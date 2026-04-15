//! Tree reconciliation engine - diffs ElementDef against live Element tree.

use crate::dirty::DirtyFlags;
use crate::element::{Element, ElementDef, SelectOption, SelectState, Tag};
use crate::id::NodeId;
use crate::layout::TextMeasureCtx;
use crate::tree::NodeArena;
use rustc_hash::FxHashMap;
use std::sync::Arc;
use taffy::TaffyTree;

/// Callbacks collected during reconciliation that must be fired after the
/// arena borrow is released. Each entry is `(node_id, on_mount_callback)`.
pub type PendingMounts = Vec<(NodeId, Arc<dyn Fn(NodeId) + Send + Sync>)>;

/// Mark ancestors of `start` with the given `flags` walking up to the root.
/// Stops early if an ancestor already carries all the requested flags
/// (meaning the propagation was already done by a prior call).
fn mark_ancestors_dirty(arena: &mut NodeArena, start: NodeId, flags: DirtyFlags) {
    let mut current = arena.get(start).map(|e| e.parent).unwrap_or(NodeId::DANGLING);
    while !current.is_dangling() {
        let already = arena.get(current).map(|e| e.dirty).unwrap_or(DirtyFlags::empty());
        if let Some(elem) = arena.get_mut(current) {
            elem.dirty |= flags;
        }
        // If all requested flags were already set, the propagation chain is
        // already complete above this node.
        if already.contains(flags) {
            break;
        }
        current = arena.get(current).map(|e| e.parent).unwrap_or(NodeId::DANGLING);
    }
}

/// Reconcile a live element tree rooted at `node_id` against a new definition.
///
/// This is the main entry point for the diffing engine. It updates the arena
/// in place, reusing existing nodes where possible and only creating or
/// destroying nodes when structurally necessary.
///
/// Returns a list of `(NodeId, on_mount_callback)` pairs that must be fired
/// by the caller after this function returns (i.e. after the arena borrow is
/// released). Firing them inside would cause a double-borrow.
pub fn reconcile(
    arena: &mut NodeArena,
    taffy: &mut TaffyTree<TextMeasureCtx>,
    node_id: NodeId,
    new_def: &ElementDef,
) -> PendingMounts {
    let mut pending_mounts: PendingMounts = Vec::new();
    reconcile_inner(arena, taffy, node_id, new_def, &mut pending_mounts);
    pending_mounts
}

fn reconcile_inner(
    arena: &mut NodeArena,
    taffy: &mut TaffyTree<TextMeasureCtx>,
    node_id: NodeId,
    new_def: &ElementDef,
    pending_mounts: &mut PendingMounts,
) {
    let existing_tag = arena.get(node_id).map(|e| e.tag);

    match existing_tag {
        Some(tag) if tag != new_def.tag => {
            // Tag changed: replace the entire subtree.
            let parent_id = arena.get(node_id).map(|e| e.parent).unwrap_or(NodeId::DANGLING);
            let prev_sib = arena.get(node_id).map(|e| e.prev_sibling).unwrap_or(NodeId::DANGLING);
            let next_sib = arena.get(node_id).map(|e| e.next_sibling).unwrap_or(NodeId::DANGLING);

            dealloc_subtree(arena, taffy, node_id);

            let new_id = build_subtree(new_def, arena, taffy, parent_id, pending_mounts);

            // Relink into the sibling chain
            if let Some(new_elem) = arena.get_mut(new_id) {
                new_elem.prev_sibling = prev_sib;
                new_elem.next_sibling = next_sib;
            }
            if !prev_sib.is_dangling() {
                if let Some(prev) = arena.get_mut(prev_sib) {
                    prev.next_sibling = new_id;
                }
            } else if !parent_id.is_dangling() {
                if let Some(parent) = arena.get_mut(parent_id) {
                    parent.first_child = new_id;
                }
            }
            if !next_sib.is_dangling() {
                if let Some(next) = arena.get_mut(next_sib) {
                    next.prev_sibling = new_id;
                }
            } else if !parent_id.is_dangling() {
                if let Some(parent) = arena.get_mut(parent_id) {
                    parent.last_child = new_id;
                }
            }
        }
        Some(_) => {
            // Before updating: check the memo fence. If the new definition carries
            // a memo_key that matches the live element's memo_key, the entire
            // subtree is considered up-to-date and we skip all diffing.
            if memo_hit(arena, node_id, new_def) {
                return;
            }
            // Tags match: update properties in place.
            update_element_properties(arena, node_id, new_def);
            // Select elements consume their option children into select_state;
            // they have no arena children to reconcile.
            if new_def.tag != Tag::Select {
                reconcile_children(arena, taffy, node_id, &new_def.children, pending_mounts);
            }
        }
        None => {
            // Node not found (stale id). Nothing to do.
        }
    }
}

/// Returns the reconciliation key for an element definition.
///
/// Prefers `def.key` (the dedicated reconciliation key) and falls back to
/// `def.id` for backward compatibility. The CSS `id` attribute continues to
/// serve double duty as a reconciliation key when no explicit `key` is set.
fn reconciliation_key(def: &ElementDef) -> Option<&str> {
    def.key.as_deref().or(def.id.as_deref())
}

/// Update an element's properties from a definition, preserving layout and
/// scroll state. Sets appropriate dirty flags and propagates SUBTREE_STYLE
/// to ancestors when the node becomes style-dirty.
///
/// Note: `key` is intentionally NOT updated here. Keys are identity, not
/// mutable properties -- the element keeps the key it was created with.
fn update_element_properties(arena: &mut NodeArena, node_id: NodeId, def: &ElementDef) {
    let Some(element) = arena.get_mut(node_id) else {
        return;
    };

    // For select elements: update options while preserving open/selection state.
    if def.tag == Tag::Select {
        let new_opts: Vec<SelectOption> = def
            .options
            .iter()
            .map(|(v, l)| SelectOption { value: v.clone(), label: l.clone() })
            .collect();
        if let Some(ref mut ss) = element.select_state {
            if ss.options != new_opts {
                ss.options = new_opts;
                element.dirty |= DirtyFlags::PAINT;
            }
        } else {
            let selected_index = def.selected_index.unwrap_or(0);
            element.select_state = Some(SelectState {
                open: false,
                selected_index,
                highlighted_index: None,
                options: new_opts,
            });
            element.dirty |= DirtyFlags::PAINT;
        }
    }

    let id_changed = element.id != def.id;
    let classes_changed = element.classes.as_slice() != def.classes.as_slice();
    let content_changed = element.content != def.content;
    let tab_index_changed = element.tab_index != def.tab_index;
    let captures_keyboard_changed = element.captures_keyboard != def.captures_keyboard;

    element.id = def.id.clone();
    element.classes = def.classes.clone();
    element.content = def.content.clone();
    element.tab_index = def.tab_index;
    element.captures_keyboard = def.captures_keyboard;
    element.on_click = def.on_click.clone();
    element.on_context_menu = def.on_context_menu.clone();
    element.on_drag = def.on_drag.clone();
    element.on_resize = def.on_resize.clone();
    element.handlers = def.handlers.clone();
    element.placeholder = def.placeholder.clone();
    element.on_change = def.on_change.clone();
    element.on_submit = def.on_submit.clone();
    // Update the memo key so subsequent rebuilds reflect the new definition.
    element.memo_key = def.memo_key;
    element.name = def.name.clone();
    // Transfer lifecycle hooks. on_mount is NOT re-fired on update (only on initial build).
    // on_unmount is updated so the latest closure fires when the node is eventually removed.
    element.on_unmount = def.on_unmount.clone();
    // Transfer inline style overrides. Since StyleDeclaration does not derive
    // PartialEq, mark LAYOUT dirty whenever either side is non-empty.
    let overrides_changed = !element.style_overrides.is_empty() || !def.style_overrides.is_empty();
    element.style_overrides = def.style_overrides.clone();
    // Transfer node_ref and refresh the stored NodeId in case the ref changed.
    if let Some(nr) = def.node_ref.clone() {
        nr.set(node_id);
        element.node_ref = Some(nr);
    }
    // Update input_type from def; preserve user state (value, cursor_pos,
    // numeric_value). checked is also preserved so toggling survives rebuilds.
    let input_type_changed = element.input_state.input_type != def.input_type;
    element.input_state.input_type = def.input_type;
    if let Some(min) = def.min {
        element.input_state.min = min;
    }
    if let Some(max) = def.max {
        element.input_state.max = max;
    }
    if let Some(step) = def.step {
        element.input_state.step = step;
    }

    let style_dirty = id_changed || classes_changed;
    let layout_dirty =
        content_changed || tab_index_changed || captures_keyboard_changed || overrides_changed;

    if style_dirty {
        element.dirty |= DirtyFlags::STYLE | DirtyFlags::PAINT;
    }
    if layout_dirty || input_type_changed {
        element.dirty |= DirtyFlags::LAYOUT | DirtyFlags::PAINT;
    }

    // Propagate SUBTREE_STYLE to ancestors so the cascade can skip clean
    // branches. We do this after setting the node's own dirty flags so the
    // ancestor walk starts from the correct node.
    if style_dirty {
        mark_ancestors_dirty(arena, node_id, DirtyFlags::SUBTREE_STYLE);
    }
    if layout_dirty {
        mark_ancestors_dirty(arena, node_id, DirtyFlags::SUBTREE_LAYOUT);
    }

    // Whenever PAINT is set on this node, propagate SUBTREE_PAINT upward so
    // the batch builder can skip clean subtrees during the render walk.
    let node_has_paint =
        arena.get(node_id).map(|e| e.dirty.contains(DirtyFlags::PAINT)).unwrap_or(false);
    if node_has_paint {
        mark_ancestors_dirty(arena, node_id, DirtyFlags::SUBTREE_PAINT);
    }
}

/// Returns true if the new definition's memo_key matches the live element's
/// memo_key, meaning the entire subtree can be skipped without any diffing.
/// Both the definition and the live element must have a memo_key set.
fn memo_hit(arena: &NodeArena, node_id: NodeId, def: &ElementDef) -> bool {
    match (def.memo_key, arena.get(node_id).and_then(|e| e.memo_key)) {
        (Some(new_key), Some(old_key)) => new_key == old_key,
        _ => false,
    }
}

/// Reconcile children of `parent_id` against a new list of child definitions.
///
/// Keyed children (those with an `id`) are matched by id via HashMap lookup.
/// Unkeyed children are matched by position. Unmatched old children are
/// deallocated, unmatched new defs get fresh subtrees.
///
/// Synthetic pseudo element children (for example a `::before` or `::after`
/// node) are pinned to the front or back of the child list respectively and
/// never participate in the match loop. After user reconciliation the
/// sibling chain is re stitched as `[leading synthetic, user children,
/// trailing synthetic]`.
fn reconcile_children(
    arena: &mut NodeArena,
    taffy: &mut TaffyTree<TextMeasureCtx>,
    parent_id: NodeId,
    new_defs: &[ElementDef],
    pending_mounts: &mut PendingMounts,
) {
    let old_children_full = arena.children(parent_id);

    // Partition old children into leading synthetic (before the first user
    // child), trailing synthetic (after the last user child), and user nodes.
    // Any synthetic nodes that appear in the middle are treated as leading
    // for stitching purposes (that situation should not occur, but being
    // defensive keeps the tree consistent).
    let mut leading_synthetic: Vec<NodeId> = Vec::new();
    let mut trailing_synthetic: Vec<NodeId> = Vec::new();
    let mut user_old: Vec<NodeId> = Vec::new();

    let mut seen_user = false;
    for &child_id in &old_children_full {
        let is_synth = arena.get(child_id).map(|e| e.synthetic).unwrap_or(false);
        if is_synth {
            if !seen_user {
                leading_synthetic.push(child_id);
            } else {
                trailing_synthetic.push(child_id);
            }
        } else {
            seen_user = true;
            user_old.push(child_id);
        }
    }

    let mut keyed_old: FxHashMap<String, NodeId> =
        FxHashMap::with_capacity_and_hasher(user_old.len(), Default::default());
    let mut unkeyed_old: Vec<NodeId> = Vec::new();

    for &child_id in &user_old {
        if let Some(element) = arena.get(child_id) {
            // Build keyed map from element's key (preferred) or id (fallback).
            let rkey = element.key.as_deref().or(element.id.as_deref());
            if let Some(k) = rkey {
                keyed_old.insert(k.to_owned(), child_id);
            } else {
                unkeyed_old.push(child_id);
            }
        }
    }

    let mut matched_user: Vec<NodeId> = Vec::with_capacity(new_defs.len());
    let mut used_old: rustc_hash::FxHashSet<NodeId> = Default::default();
    let mut unkeyed_idx = 0usize;

    for def in new_defs {
        let mut found = false;

        if let Some(rkey) = reconciliation_key(def) {
            if let Some(&old_id) = keyed_old.get(rkey) {
                // Keyed match: reconcile handles memo check internally.
                reconcile_inner(arena, taffy, old_id, def, pending_mounts);
                matched_user.push(old_id);
                used_old.insert(old_id);
                found = true;
            }
        } else {
            // Unkeyed: match by position
            while unkeyed_idx < unkeyed_old.len() {
                let candidate = unkeyed_old[unkeyed_idx];
                unkeyed_idx += 1;
                if !used_old.contains(&candidate) {
                    // reconcile handles memo check internally.
                    reconcile_inner(arena, taffy, candidate, def, pending_mounts);
                    matched_user.push(candidate);
                    used_old.insert(candidate);
                    found = true;
                    break;
                }
            }
        }

        if !found {
            // No match found: create new subtree and mark ancestors dirty
            // because this is a structural change.
            let new_id = build_subtree(def, arena, taffy, parent_id, pending_mounts);
            mark_ancestors_dirty(
                arena,
                new_id,
                DirtyFlags::SUBTREE_STYLE | DirtyFlags::SUBTREE_LAYOUT | DirtyFlags::SUBTREE_PAINT,
            );
            matched_user.push(new_id);
        }
    }

    for &old_id in &user_old {
        if !used_old.contains(&old_id) {
            // Removing a child is a structural change; mark ancestors.
            mark_ancestors_dirty(
                arena,
                old_id,
                DirtyFlags::SUBTREE_STYLE | DirtyFlags::SUBTREE_LAYOUT | DirtyFlags::SUBTREE_PAINT,
            );
            dealloc_subtree(arena, taffy, old_id);
        }
    }

    // Re stitch the full sibling chain in pinned order: leading synthetic,
    // matched user children, trailing synthetic.
    let mut final_children: Vec<NodeId> =
        Vec::with_capacity(leading_synthetic.len() + matched_user.len() + trailing_synthetic.len());
    final_children.extend_from_slice(&leading_synthetic);
    final_children.extend_from_slice(&matched_user);
    final_children.extend_from_slice(&trailing_synthetic);

    let children_changed = final_children.len() != old_children_full.len()
        || final_children != old_children_full.as_ref();

    if !children_changed {
        return;
    }

    if let Some(parent) = arena.get_mut(parent_id) {
        if final_children.is_empty() {
            parent.first_child = NodeId::DANGLING;
            parent.last_child = NodeId::DANGLING;
        } else {
            parent.first_child = final_children[0];
            parent.last_child = final_children[final_children.len() - 1];
        }
        parent.dirty |= DirtyFlags::CHILDREN;
    }

    for i in 0..final_children.len() {
        let child_id = final_children[i];
        let prev = if i > 0 { final_children[i - 1] } else { NodeId::DANGLING };
        let next =
            if i + 1 < final_children.len() { final_children[i + 1] } else { NodeId::DANGLING };

        if let Some(child) = arena.get_mut(child_id) {
            child.parent = parent_id;
            child.prev_sibling = prev;
            child.next_sibling = next;
        }
    }
}

/// Build a full subtree from a definition, similar to `build_tree_from_def`.
#[allow(clippy::only_used_in_recursion)]
pub fn build_subtree(
    def: &ElementDef,
    arena: &mut NodeArena,
    taffy: &mut TaffyTree<TextMeasureCtx>,
    parent: NodeId,
    pending_mounts: &mut PendingMounts,
) -> NodeId {
    let mut element = Element::new(def.tag);
    element.parent = parent;
    element.classes = def.classes.clone();
    element.id = def.id.clone();
    element.key = def.key.clone();
    element.content = def.content.clone();
    element.on_click = def.on_click.clone();
    element.tab_index = def.tab_index;
    element.captures_keyboard = def.captures_keyboard;
    element.on_context_menu = def.on_context_menu.clone();
    element.on_drag = def.on_drag.clone();
    element.on_resize = def.on_resize.clone();
    element.handlers = def.handlers.clone();
    element.placeholder = def.placeholder.clone();
    element.on_change = def.on_change.clone();
    element.on_submit = def.on_submit.clone();
    element.memo_key = def.memo_key;
    element.name = def.name.clone();
    element.input_state.input_type = def.input_type;
    if let Some(min) = def.min {
        element.input_state.min = min;
    }
    if let Some(max) = def.max {
        element.input_state.max = max;
    }
    if let Some(step) = def.step {
        element.input_state.step = step;
    }
    element.input_state.checked = def.checked;
    element.on_mount = def.on_mount.clone();
    element.on_unmount = def.on_unmount.clone();
    element.style_overrides = def.style_overrides.clone();

    // For select elements: populate SelectState from the def's options list.
    // Option-tag children are consumed here and NOT added to the arena tree.
    if def.tag == Tag::Select {
        let options: Vec<SelectOption> = def
            .options
            .iter()
            .map(|(v, l)| SelectOption { value: v.clone(), label: l.clone() })
            .collect();
        let selected_index = def.selected_index.unwrap_or(0);
        element.select_state =
            Some(SelectState { open: false, selected_index, highlighted_index: None, options });
    }

    let node_id = arena.alloc(element);

    // If the def carries a NodeRef, store it on the element and record the id.
    if let Some(nr) = def.node_ref.clone() {
        nr.set(node_id);
        if let Some(elem) = arena.get_mut(node_id) {
            elem.node_ref = Some(nr);
        }
    }

    // Collect mount callback to be fired by the caller after reconciliation.
    if let Some(cb) = def.on_mount.clone() {
        pending_mounts.push((node_id, cb));
    }

    // For select elements, do not add option children as arena nodes.
    if def.tag == Tag::Select {
        return node_id;
    }

    let mut prev_child = NodeId::DANGLING;
    for child_def in &def.children {
        // Skip Tag::Option children at any level (they are consumed by the select)
        if child_def.tag == Tag::Option {
            continue;
        }
        let child_id = build_subtree(child_def, arena, taffy, node_id, pending_mounts);

        if let Some(child) = arena.get_mut(child_id) {
            child.prev_sibling = prev_child;
        }

        if prev_child.is_dangling() {
            if let Some(parent_elem) = arena.get_mut(node_id) {
                parent_elem.first_child = child_id;
            }
        } else if let Some(prev) = arena.get_mut(prev_child) {
            prev.next_sibling = child_id;
        }

        if let Some(parent_elem) = arena.get_mut(node_id) {
            parent_elem.last_child = child_id;
        }

        prev_child = child_id;
    }

    node_id
}

/// Recursively deallocate a subtree rooted at `node_id`, including taffy nodes.
fn dealloc_subtree(arena: &mut NodeArena, taffy: &mut TaffyTree<TextMeasureCtx>, node_id: NodeId) {
    let children = arena.children(node_id);
    for child_id in children {
        dealloc_subtree(arena, taffy, child_id);
    }

    // Clone the callbacks and ref handle out before dealloc to avoid borrow-after-free.
    let unmount_cb = arena.get(node_id).and_then(|e| e.on_unmount.clone());
    let node_ref = arena.get(node_id).and_then(|e| e.node_ref.clone());
    let taffy_node = arena.get(node_id).and_then(|e| e.taffy_node);

    // Clear the NodeRef before the node is removed so callers see None immediately.
    if let Some(nr) = node_ref {
        nr.clear();
    }

    if let Some(taffy_node) = taffy_node {
        let _ = taffy.remove(taffy_node);
    }

    arena.dealloc(node_id);

    // Fire unmount after dealloc so the node is no longer in the tree.
    if let Some(cb) = unmount_cb {
        cb(node_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{ElementDef, Tag};
    use crate::tree::NodeArena;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use taffy::TaffyTree;

    #[test]
    fn on_mount_fires_on_build_subtree() {
        let count = Arc::new(AtomicU32::new(0));
        let count2 = Arc::clone(&count);
        let def = ElementDef::new(Tag::Div).on_mount(move |_id| {
            count2.fetch_add(1, Ordering::SeqCst);
        });

        let mut arena = NodeArena::new();
        let mut taffy = TaffyTree::new();
        let mut pending = Vec::new();
        build_subtree(&def, &mut arena, &mut taffy, NodeId::DANGLING, &mut pending);
        for (id, cb) in pending {
            cb(id);
        }

        assert_eq!(count.load(Ordering::SeqCst), 1, "on_mount should fire once on first build");
    }

    #[test]
    fn on_unmount_fires_on_dealloc_subtree() {
        let count = Arc::new(AtomicU32::new(0));
        let count2 = Arc::clone(&count);
        let def = ElementDef::new(Tag::Div).on_unmount(move |_id| {
            count2.fetch_add(1, Ordering::SeqCst);
        });

        let mut arena = NodeArena::new();
        let mut taffy = TaffyTree::new();
        let mut pending = Vec::new();
        let root_id = build_subtree(&def, &mut arena, &mut taffy, NodeId::DANGLING, &mut pending);
        for (id, cb) in pending {
            cb(id);
        }

        assert_eq!(count.load(Ordering::SeqCst), 0, "on_unmount should not fire before dealloc");
        dealloc_subtree(&mut arena, &mut taffy, root_id);
        assert_eq!(count.load(Ordering::SeqCst), 1, "on_unmount should fire once on dealloc");
    }

    #[test]
    fn on_mount_does_not_refire_on_update() {
        let mount_count = Arc::new(AtomicU32::new(0));
        let mount_count2 = Arc::clone(&mount_count);

        let def = ElementDef::new(Tag::Div).on_mount(move |_id| {
            mount_count2.fetch_add(1, Ordering::SeqCst);
        });

        let mut arena = NodeArena::new();
        let mut taffy = TaffyTree::new();
        let mut pending = Vec::new();
        let root_id = build_subtree(&def, &mut arena, &mut taffy, NodeId::DANGLING, &mut pending);
        for (id, cb) in pending {
            cb(id);
        }

        assert_eq!(mount_count.load(Ordering::SeqCst), 1);

        // Reconcile again with the same def -- same tag, so update path taken.
        let def2 = ElementDef::new(Tag::Div).on_mount(move |_id| {
            // This closure should never fire on update.
            panic!("on_mount should not fire on update");
        });
        let pending2 = reconcile(&mut arena, &mut taffy, root_id, &def2);
        for (id, cb) in pending2 {
            cb(id);
        }

        assert_eq!(mount_count.load(Ordering::SeqCst), 1, "on_mount must not re-fire on update");
    }

    #[test]
    fn both_fire_on_tag_change_replacement() {
        let mount_count = Arc::new(AtomicU32::new(0));
        let unmount_count = Arc::new(AtomicU32::new(0));

        let mc2 = Arc::clone(&mount_count);
        let uc2 = Arc::clone(&unmount_count);

        // Build initial div with on_unmount.
        let old_def = ElementDef::new(Tag::Div).on_unmount(move |_id| {
            uc2.fetch_add(1, Ordering::SeqCst);
        });

        let mut arena = NodeArena::new();
        let mut taffy = TaffyTree::new();
        let mut pending = Vec::new();
        let root_id =
            build_subtree(&old_def, &mut arena, &mut taffy, NodeId::DANGLING, &mut pending);
        for (id, cb) in pending {
            cb(id);
        }

        // Reconcile to a span (tag change) with on_mount.
        let new_def = ElementDef::new(Tag::Span).on_mount(move |_id| {
            mc2.fetch_add(1, Ordering::SeqCst);
        });

        // Tag change path: need to set parent/sibling for relinking to work.
        // Since root has no parent, we simulate by just calling reconcile_inner directly.
        let mut pending2: PendingMounts = Vec::new();
        reconcile_inner(&mut arena, &mut taffy, root_id, &new_def, &mut pending2);
        for (id, cb) in pending2 {
            cb(id);
        }

        assert_eq!(
            unmount_count.load(Ordering::SeqCst),
            1,
            "old on_unmount must fire on tag replace"
        );
        assert_eq!(mount_count.load(Ordering::SeqCst), 1, "new on_mount must fire on tag replace");
    }

    // NodeRef integration tests

    #[test]
    fn node_ref_set_on_build() {
        use crate::id::NodeRef;

        let nr = NodeRef::new();
        let def = ElementDef::new(Tag::Div).with_ref(nr.clone());

        let mut arena = NodeArena::new();
        let mut taffy = TaffyTree::new();
        let mut pending = Vec::new();
        let root_id = build_subtree(&def, &mut arena, &mut taffy, NodeId::DANGLING, &mut pending);

        let stored = nr.get().expect("NodeRef should be set after build_subtree");
        assert_eq!(stored, root_id, "NodeRef must point to the allocated NodeId");
    }

    #[test]
    fn node_ref_set_on_update() {
        use crate::id::NodeRef;

        let nr = NodeRef::new();
        let def = ElementDef::new(Tag::Div);

        let mut arena = NodeArena::new();
        let mut taffy = TaffyTree::new();
        let mut pending = Vec::new();
        let root_id = build_subtree(&def, &mut arena, &mut taffy, NodeId::DANGLING, &mut pending);

        // Initially no ref stored.
        assert!(nr.get().is_none());

        // Reconcile with a def that carries the ref.
        let def2 = ElementDef::new(Tag::Div).with_ref(nr.clone());
        let pending2 = reconcile(&mut arena, &mut taffy, root_id, &def2);
        for (id, cb) in pending2 {
            cb(id);
        }

        let stored = nr.get().expect("NodeRef should be set after update");
        assert_eq!(stored, root_id, "NodeRef must point to the same NodeId after update");
    }

    #[test]
    fn node_ref_cleared_on_dealloc() {
        use crate::id::NodeRef;

        let nr = NodeRef::new();
        let def = ElementDef::new(Tag::Div).with_ref(nr.clone());

        let mut arena = NodeArena::new();
        let mut taffy = TaffyTree::new();
        let mut pending = Vec::new();
        let root_id = build_subtree(&def, &mut arena, &mut taffy, NodeId::DANGLING, &mut pending);

        assert!(nr.get().is_some(), "NodeRef must be set after mount");

        dealloc_subtree(&mut arena, &mut taffy, root_id);

        assert!(nr.get().is_none(), "NodeRef must be cleared after dealloc");
    }

    // Damage-aware rendering tests (SUBTREE_PAINT propagation)

    /// Build a two-level tree: root div containing one child div.
    fn setup_two_level() -> (NodeArena, taffy::TaffyTree<TextMeasureCtx>, NodeId, NodeId) {
        let mut arena = NodeArena::new();
        let mut taffy = taffy::TaffyTree::<TextMeasureCtx>::new();
        let mut pending = Vec::new();

        let parent_def = ElementDef::new(Tag::Div);
        let child_def = ElementDef::new(Tag::Div).with_class("child");

        let parent_id =
            build_subtree(&parent_def, &mut arena, &mut taffy, NodeId::DANGLING, &mut pending);
        let child_id = build_subtree(&child_def, &mut arena, &mut taffy, parent_id, &mut pending);

        // Link child into parent
        if let Some(parent) = arena.get_mut(parent_id) {
            parent.first_child = child_id;
            parent.last_child = child_id;
        }

        (arena, taffy, parent_id, child_id)
    }

    #[test]
    fn setting_paint_on_leaf_propagates_subtree_paint_to_parent() {
        let (mut arena, _, parent_id, child_id) = setup_two_level();

        // Manually set PAINT on the child (simulates what happens during reconcile).
        arena.get_mut(child_id).unwrap().dirty |= DirtyFlags::PAINT;

        // Propagate SUBTREE_PAINT to ancestors.
        mark_ancestors_dirty(&mut arena, child_id, DirtyFlags::SUBTREE_PAINT);

        let parent_dirty = arena.get(parent_id).unwrap().dirty;
        assert!(
            parent_dirty.contains(DirtyFlags::SUBTREE_PAINT),
            "parent should carry SUBTREE_PAINT when child has PAINT set; got {:?}",
            parent_dirty
        );
    }

    #[test]
    fn paint_flag_on_leaf_does_not_pollute_parent_with_plain_paint() {
        let (mut arena, _, parent_id, child_id) = setup_two_level();

        // Clear parent flags to simulate a previously clean frame.
        arena.get_mut(parent_id).unwrap().dirty = DirtyFlags::empty();
        arena.get_mut(child_id).unwrap().dirty |= DirtyFlags::PAINT;
        mark_ancestors_dirty(&mut arena, child_id, DirtyFlags::SUBTREE_PAINT);

        let parent_dirty = arena.get(parent_id).unwrap().dirty;
        // The parent should NOT have PAINT itself, only SUBTREE_PAINT.
        assert!(
            !parent_dirty.contains(DirtyFlags::PAINT),
            "parent should not have PAINT (only SUBTREE_PAINT); got {:?}",
            parent_dirty
        );
    }

    #[test]
    fn new_subtree_insertion_propagates_subtree_paint_to_ancestors() {
        let mut arena = NodeArena::new();
        let mut taffy = taffy::TaffyTree::<TextMeasureCtx>::new();
        let mut pending = Vec::new();

        let parent_def = ElementDef::new(Tag::Div);
        let parent_id =
            build_subtree(&parent_def, &mut arena, &mut taffy, NodeId::DANGLING, &mut pending);

        // Clear all dirty flags on parent to simulate a previously clean frame.
        arena.get_mut(parent_id).unwrap().dirty = DirtyFlags::empty();

        // Simulate inserting a new child (as reconcile_children does).
        let child_def = ElementDef::new(Tag::Div);
        let child_id = build_subtree(&child_def, &mut arena, &mut taffy, parent_id, &mut pending);
        mark_ancestors_dirty(
            &mut arena,
            child_id,
            DirtyFlags::SUBTREE_STYLE | DirtyFlags::SUBTREE_LAYOUT | DirtyFlags::SUBTREE_PAINT,
        );

        let parent_dirty = arena.get(parent_id).unwrap().dirty;
        assert!(
            parent_dirty.contains(DirtyFlags::SUBTREE_PAINT),
            "parent should carry SUBTREE_PAINT after new child insertion; got {:?}",
            parent_dirty
        );
    }
}
