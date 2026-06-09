//! Shared tree-building helpers used by app, test-harness, and benchmarks.

use crate::dirty::DirtyFlags;
use crate::element::{Element, ElementDef, SelectOption, SelectState, Tag};
use crate::id::NodeId;
use crate::layout::{self, TextMeasureCache, TextMeasureCtx};
use crate::style::animation::AnimationDriver;
use crate::style::cascade;
use crate::style::parse::CompiledStylesheet;
use crate::style::parse::ScopeKey;
use crate::style::pseudo::{self, PseudoSideTable};
use crate::style::transition::{self, ActiveTransitions};
use crate::style::types::Dimension;
use crate::tree::NodeArena;
use cosmic_text::FontSystem;
use std::time::Instant;

/// Recursively build an arena tree from an [`ElementDef`], linking parent/child/sibling pointers.
#[allow(clippy::only_used_in_recursion)]
pub fn build_tree_from_def(
    def: &ElementDef,
    arena: &mut NodeArena,
    taffy: &mut taffy::TaffyTree<TextMeasureCtx>,
    parent: NodeId,
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
    element.resize_axis = def.resize_axis;
    element.on_pane_resize = def.on_pane_resize.clone();
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

    // For select elements: populate SelectState from the def's options list.
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

    // For select elements, do not add option children as arena nodes.
    if def.tag == Tag::Select {
        return node_id;
    }

    let mut prev_child = NodeId::DANGLING;
    for child_def in &def.children {
        // Skip Tag::Option children (they are consumed by the select's state)
        if child_def.tag == Tag::Option {
            continue;
        }
        let child_id = build_tree_from_def(child_def, arena, taffy, node_id);

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

/// Cascade-resolve styles for every node in the subtree rooted at `node_id`.
pub fn resolve_all_styles(
    arena: &mut NodeArena,
    stylesheet: &CompiledStylesheet,
    node_id: NodeId,
    hovered: NodeId,
    active: Option<NodeId>,
    focused: NodeId,
) {
    resolve_all_styles_with_transitions(
        arena, stylesheet, node_id, hovered, active, focused, false, None, None,
    );
}

/// Resolve pseudo element content for the subtree rooted at `node_id`.
///
/// Call this after `resolve_all_styles_with_transitions` and before the
/// layout sync pass so synthetic ::before and ::after nodes show up in
/// taffy as normal children of their host. The `pseudo_table` side table
/// must be preserved across frames so nodes can be updated in place instead
/// of reallocated.
#[allow(clippy::too_many_arguments)]
pub fn resolve_pseudo_elements(
    arena: &mut NodeArena,
    taffy: &mut taffy::TaffyTree<TextMeasureCtx>,
    stylesheet: &CompiledStylesheet,
    node_id: NodeId,
    hovered: NodeId,
    active: Option<NodeId>,
    focused: NodeId,
    pseudo_table: &mut PseudoSideTable,
) {
    pseudo::resolve_pseudo_elements(
        arena,
        taffy,
        stylesheet,
        node_id,
        hovered,
        active,
        focused,
        pseudo_table,
    );
}

/// Cascade-resolve styles with transition support.
///
/// When `now` and `active_transitions` are provided, this will diff the old
/// and new styles and start transitions for properties that changed (if the
/// element's CSS specifies `transition`).
///
/// This performs a **full** cascade: every node in the subtree is processed.
/// Use this when hover/focus/active state has changed, since pseudo-class
/// selectors can match any node in the tree.
///
/// For post-reconcile cascades where only specific nodes changed, use
/// `resolve_dirty_styles_with_transitions` which short-circuits clean subtrees.
#[allow(clippy::too_many_arguments)]
pub fn resolve_all_styles_with_transitions(
    arena: &mut NodeArena,
    stylesheet: &CompiledStylesheet,
    node_id: NodeId,
    hovered: NodeId,
    active: Option<NodeId>,
    focused: NodeId,
    focus_via_keyboard: bool,
    now: Option<Instant>,
    active_transitions: Option<&mut ActiveTransitions>,
) {
    // The active root theme scope is a property of the document root, so it is
    // identical for every node in this pass; compute it once at the entry and
    // thread it through the recursion (O(1) per element thereafter). Cheap
    // (`None`) for stylesheets without theme scopes.
    let active_root_scope = cascade::active_root_scope_for(arena, stylesheet, node_id);
    resolve_all_styles_with_transitions_scoped(
        arena,
        stylesheet,
        node_id,
        hovered,
        active,
        focused,
        focus_via_keyboard,
        now,
        active_transitions,
        active_root_scope,
    );
}

/// Recursion body of [`resolve_all_styles_with_transitions`]. Carries the
/// pass-wide `active_root_scope` so it is not recomputed per node.
#[allow(clippy::too_many_arguments)]
fn resolve_all_styles_with_transitions_scoped(
    arena: &mut NodeArena,
    stylesheet: &CompiledStylesheet,
    node_id: NodeId,
    hovered: NodeId,
    active: Option<NodeId>,
    focused: NodeId,
    focus_via_keyboard: bool,
    now: Option<Instant>,
    mut active_transitions: Option<&mut ActiveTransitions>,
    active_root_scope: Option<ScopeKey>,
) {
    // Anonymous text boxes are invisible to the cascade: their style is
    // derived from their host's computed style by the layout sync (the
    // single writer; see `layout::sync_anonymous_text_child`). Rule-matching
    // them here would stomp the derivation with `span`/`*` matches and
    // could even register transitions on them.
    if arena.get(node_id).map(|e| e.anonymous).unwrap_or(false) {
        return;
    }
    let mut resolved_style = cascade::resolve_style_fv(
        arena,
        stylesheet,
        node_id,
        hovered,
        active,
        focused,
        focus_via_keyboard,
        active_root_scope,
    );
    let sel_style =
        cascade::resolve_selection_style(arena, stylesheet, node_id, hovered, active, focused);
    let children = arena.children(node_id);

    if let Some(element) = arena.get_mut(node_id) {
        // Apply inline and runtime overrides before diffing or starting
        // transitions so comparisons happen on the final resolved style.
        for decl in &element.style_overrides {
            crate::style::parse::apply_declaration(&mut resolved_style, decl);
        }
        if let Some(w) = element.resize_override_width {
            resolved_style.width = Dimension::Px(w);
        }
        if let Some(h) = element.resize_override_height {
            resolved_style.height = Dimension::Px(h);
        }
        resolved_style.apply_font_size_scale();

        // If the new style declares transitions and we have a previous style to diff against,
        // start transitions for changed properties.
        if let (Some(now), true) = (now, !resolved_style.transitions.is_empty()) {
            if let Some(ref prev) = element.previous_style {
                transition::start_transitions(
                    prev,
                    &resolved_style,
                    &resolved_style.transitions,
                    &mut element.running_transitions,
                    now,
                );
            }
        }

        element.previous_style = Some(Box::new(resolved_style.clone()));
        element.computed_style = resolved_style;
        element.selection_style = sel_style;

        // Clear style dirty flags now that this node has been processed.
        element.dirty.remove(DirtyFlags::STYLE | DirtyFlags::SUBTREE_STYLE);

        // Always mark paint-dirty AND layout-dirty after restyle. A targeted
        // comparison (visual_props_changed) is defeated by DPI scaling:
        // scale_all_styles mutates computed_style after restyle, so the "old"
        // style is scaled while the "new" cascade result is unscaled, causing
        // a false diff on every frame. Unconditional PAINT is safe here
        // because the batch cache replay fix ensures idle frames carry
        // forward correctly. LAYOUT must be set too because style changes can
        // affect size/position (display, flex-direction, width, padding,
        // etc.), and `sync_element_to_taffy` only re-syncs the taffy node
        // when LAYOUT is dirty.
        element.dirty.insert(DirtyFlags::PAINT | DirtyFlags::LAYOUT);
        if !children.is_empty() {
            element.dirty.insert(DirtyFlags::SUBTREE_PAINT | DirtyFlags::SUBTREE_LAYOUT);
        }
    }

    // Track elements with active transitions.
    if let Some(ref mut at) = active_transitions {
        if let Some(element) = arena.get(node_id) {
            if !element.running_transitions.is_empty() {
                at.add(node_id);
            }
        }
    }

    // We need to reborrow for recursion since active_transitions is &mut.
    for &child_id in &children {
        // We can't pass `active_transitions` directly in a loop due to borrow rules,
        // so we use a raw pointer trick or just handle it differently.
        // Actually, Option<&mut T> can be reborrowed:
        resolve_all_styles_with_transitions_scoped(
            arena,
            stylesheet,
            child_id,
            hovered,
            active,
            focused,
            focus_via_keyboard,
            now,
            None, // Children track themselves individually.
            active_root_scope,
        );
        // After resolving child, check if it has active transitions and track it.
        if let Some(ref mut at) = active_transitions {
            collect_active_transitions_subtree(arena, child_id, at);
        }
    }

    let child_paint_dirty = children.iter().any(|&child_id| {
        arena
            .get(child_id)
            .map(|child| child.dirty.intersects(DirtyFlags::PAINT | DirtyFlags::SUBTREE_PAINT))
            .unwrap_or(false)
    });

    if let Some(element) = arena.get_mut(node_id) {
        if child_paint_dirty {
            element.dirty.insert(DirtyFlags::SUBTREE_PAINT);
        } else {
            element.dirty.remove(DirtyFlags::SUBTREE_PAINT);
        }
    }
}

/// Cascade-resolve styles using dirty-flag short-circuiting.
///
/// Only processes nodes that have `STYLE` dirty, and only descends into
/// subtrees that have `SUBTREE_STYLE` dirty. This is safe to call after
/// reconciliation because the reconciler sets these flags precisely on the
/// nodes that changed and propagates `SUBTREE_STYLE` up to all ancestors.
///
/// On the initial build every node has `STYLE` set by `Element::new`, so
/// every node is processed on the first pass. On subsequent frames where
/// only specific nodes were reconciled, the cascade skips clean subtrees.
///
/// Do NOT use this when hover/focus/active state has changed because
/// pseudo-class selectors can match any node in the tree; use
/// `resolve_all_styles_with_transitions` in that case.
#[allow(clippy::too_many_arguments)]
pub fn resolve_dirty_styles_with_transitions(
    arena: &mut NodeArena,
    stylesheet: &CompiledStylesheet,
    node_id: NodeId,
    hovered: NodeId,
    active: Option<NodeId>,
    focused: NodeId,
    focus_via_keyboard: bool,
    now: Option<Instant>,
    active_transitions: Option<&mut ActiveTransitions>,
) {
    // Active root theme scope is pass-wide (a property of the document root);
    // compute once at the entry and thread through recursion.
    let active_root_scope = cascade::active_root_scope_for(arena, stylesheet, node_id);
    resolve_dirty_styles_with_transitions_scoped(
        arena,
        stylesheet,
        node_id,
        hovered,
        active,
        focused,
        focus_via_keyboard,
        now,
        active_transitions,
        active_root_scope,
    );
}

/// Recursion body of [`resolve_dirty_styles_with_transitions`]. Carries the
/// pass-wide `active_root_scope` so it is not recomputed per node.
#[allow(clippy::too_many_arguments)]
fn resolve_dirty_styles_with_transitions_scoped(
    arena: &mut NodeArena,
    stylesheet: &CompiledStylesheet,
    node_id: NodeId,
    hovered: NodeId,
    active: Option<NodeId>,
    focused: NodeId,
    focus_via_keyboard: bool,
    now: Option<Instant>,
    mut active_transitions: Option<&mut ActiveTransitions>,
    active_root_scope: Option<ScopeKey>,
) {
    // Anonymous text boxes are invisible to the cascade (see the matching
    // skip in `resolve_all_styles_with_transitions_scoped`).
    if arena.get(node_id).map(|e| e.anonymous).unwrap_or(false) {
        return;
    }
    // Short-circuit: if this node has no style work anywhere in its subtree,
    // skip it entirely.
    let needs_style_work = arena
        .get(node_id)
        .map(|e| e.dirty.intersects(DirtyFlags::STYLE | DirtyFlags::SUBTREE_STYLE))
        .unwrap_or(false);

    if !needs_style_work {
        // Still collect any active transitions that were already running.
        if let Some(ref mut at) = active_transitions {
            collect_active_transitions_subtree(arena, node_id, at);
        }
        return;
    }

    // Recompute this node's own style only when the node itself is dirty.
    let node_style_dirty =
        arena.get(node_id).map(|e| e.dirty.contains(DirtyFlags::STYLE)).unwrap_or(false);

    let resolved_style = if node_style_dirty {
        Some(cascade::resolve_style_fv(
            arena,
            stylesheet,
            node_id,
            hovered,
            active,
            focused,
            focus_via_keyboard,
            active_root_scope,
        ))
    } else {
        None
    };
    let sel_style = if node_style_dirty {
        cascade::resolve_selection_style(arena, stylesheet, node_id, hovered, active, focused)
    } else {
        None
    };

    let children = arena.children(node_id);

    if let Some(mut resolved_style) = resolved_style {
        if let Some(element) = arena.get_mut(node_id) {
            for decl in &element.style_overrides {
                crate::style::parse::apply_declaration(&mut resolved_style, decl);
            }
            if let Some(w) = element.resize_override_width {
                resolved_style.width = Dimension::Px(w);
            }
            if let Some(h) = element.resize_override_height {
                resolved_style.height = Dimension::Px(h);
            }
            resolved_style.apply_font_size_scale();

            // If the new style declares transitions and we have a previous style to diff against,
            // start transitions for changed properties.
            if let (Some(now), true) = (now, !resolved_style.transitions.is_empty()) {
                if let Some(ref prev) = element.previous_style {
                    transition::start_transitions(
                        prev,
                        &resolved_style,
                        &resolved_style.transitions,
                        &mut element.running_transitions,
                        now,
                    );
                }
            }

            element.previous_style = Some(Box::new(resolved_style.clone()));
            element.computed_style = resolved_style;
            element.selection_style = sel_style;

            // Clear the node's own STYLE flag now that it has been resolved.
            element.dirty.remove(DirtyFlags::STYLE);

            // Unconditional PAINT and LAYOUT (see
            // resolve_all_styles_with_transitions for rationale).
            element.dirty.insert(DirtyFlags::PAINT | DirtyFlags::LAYOUT);
            if !children.is_empty() {
                element.dirty.insert(DirtyFlags::SUBTREE_PAINT | DirtyFlags::SUBTREE_LAYOUT);
            }
        }
    }

    // Track elements with active transitions.
    if let Some(ref mut at) = active_transitions {
        if let Some(element) = arena.get(node_id) {
            if !element.running_transitions.is_empty() {
                at.add(node_id);
            }
        }
    }

    // We need to reborrow for recursion since active_transitions is &mut.
    for &child_id in &children {
        resolve_dirty_styles_with_transitions_scoped(
            arena,
            stylesheet,
            child_id,
            hovered,
            active,
            focused,
            focus_via_keyboard,
            now,
            None, // Children track themselves individually.
            active_root_scope,
        );
        // After resolving child, check if it has active transitions and track it.
        if let Some(ref mut at) = active_transitions {
            collect_active_transitions_subtree(arena, child_id, at);
        }
    }

    let child_paint_dirty = children.iter().any(|&child_id| {
        arena
            .get(child_id)
            .map(|child| child.dirty.intersects(DirtyFlags::PAINT | DirtyFlags::SUBTREE_PAINT))
            .unwrap_or(false)
    });

    if let Some(element) = arena.get_mut(node_id) {
        if child_paint_dirty {
            element.dirty.insert(DirtyFlags::SUBTREE_PAINT);
        } else {
            element.dirty.remove(DirtyFlags::SUBTREE_PAINT);
        }
    }

    // After processing all children, clear the SUBTREE_STYLE flag so future
    // passes know this subtree is clean.
    if let Some(element) = arena.get_mut(node_id) {
        element.dirty.remove(DirtyFlags::SUBTREE_STYLE);
    }
}

/// Collect all elements with active transitions in a subtree into the tracker.
fn collect_active_transitions_subtree(
    arena: &NodeArena,
    node_id: NodeId,
    at: &mut ActiveTransitions,
) {
    if let Some(element) = arena.get(node_id) {
        if !element.running_transitions.is_empty() {
            at.add(node_id);
        }
        let mut child = element.first_child;
        while !child.is_dangling() {
            collect_active_transitions_subtree(arena, child, at);
            child = arena.get(child).map(|e| e.next_sibling).unwrap_or(NodeId::DANGLING);
        }
    }
}

/// Tick all active transitions in the arena: interpolate values, apply to
/// computed styles, remove completed transitions. Returns true if any
/// transitions are still active.
pub fn tick_all_transitions(
    arena: &mut NodeArena,
    active: &mut ActiveTransitions,
    now: Instant,
) -> bool {
    let mut ticked_nodes: smallvec::SmallVec<[NodeId; 8]> = smallvec::SmallVec::new();

    let mut i = 0;
    while i < active.nodes.len() {
        let node_id = active.nodes[i];
        if let Some(element) = arena.get_mut(node_id) {
            let still_active = transition::tick_transitions(
                &mut element.computed_style,
                &mut element.running_transitions,
                now,
            );
            ticked_nodes.push(node_id);
            if !still_active {
                active.nodes.swap_remove(i);
                // don't increment i
                continue;
            }
        } else {
            // Node was deallocated.
            active.nodes.swap_remove(i);
            continue;
        }
        i += 1;
    }

    // Mark ticked nodes paint-dirty so the batch builder re-renders them
    // with the interpolated style values.
    for &nid in &ticked_nodes {
        mark_node_paint_dirty(arena, nid);
        // Ticks run on PAINT-only frames where the layout sync (the normal
        // derivation point for anonymous text boxes) never runs; refresh
        // the box's style copy here or its color/opacity/font would freeze
        // at the value captured on the last layout frame.
        crate::layout::refresh_anon_text_style(arena, nid);
    }

    active.has_active()
}

/// Walk the arena and push each element's current animation list into the
/// driver's side table.
///
/// The driver stores animation state per node, so this pass is what
/// actually starts new animations and removes stale ones. It must run after
/// `resolve_all_styles_with_transitions` so the cascaded animation list is
/// up to date on each element.
///
/// The current `computed_style` is captured as the base style whenever a
/// new animation state is created, so the driver can synthesize missing
/// keyframe endpoints without reading back its own previous output.
pub fn sync_all_animations(
    arena: &NodeArena,
    driver: &mut AnimationDriver,
    node_id: NodeId,
    now: Instant,
) {
    if let Some(element) = arena.get(node_id) {
        driver.sync_node(node_id, &element.computed_style.animations, &element.computed_style, now);
        let mut child = element.first_child;
        while !child.is_dangling() {
            sync_all_animations(arena, driver, child, now);
            child = arena.get(child).map(|e| e.next_sibling).unwrap_or(NodeId::DANGLING);
        }
    }
}

/// Tick every running animation in the driver and apply the sampled values
/// back onto each element's computed style. Delegates to
/// `AnimationDriver::tick` but keeps the symmetry with `tick_all_transitions`
/// so the app crate can call either independently.
pub fn tick_all_animations(
    arena: &mut NodeArena,
    driver: &mut AnimationDriver,
    stylesheet: &crate::style::parse::CompiledStylesheet,
    now: Instant,
) {
    let ticked = driver.tick(arena, stylesheet, now);
    for &nid in &ticked {
        mark_node_paint_dirty(arena, nid);
        // Same anonymous-text-box refresh as in `tick_all_transitions`.
        crate::layout::refresh_anon_text_style(arena, nid);
    }
}

/// Apply DPI scaling to every computed style in the subtree.
pub fn scale_all_styles(arena: &mut NodeArena, node_id: NodeId, scale: f32) {
    if (scale - 1.0).abs() < 0.001 {
        return;
    }

    let children = arena.children(node_id);

    if let Some(element) = arena.get_mut(node_id) {
        // Anonymous text boxes copy their host's ALREADY-SCALED style in
        // the layout sync; scaling the stored copy here would double-scale.
        if element.anonymous {
            return;
        }
        element.computed_style.scale_by(scale);
    }

    for child_id in children {
        scale_all_styles(arena, child_id, scale);
    }
}

/// Mark a single node as PAINT-dirty and propagate SUBTREE_PAINT up the
/// ancestor chain. Use this when a node's visual output changed without
/// going through the reconciler (e.g., transition/animation ticks).
pub fn mark_node_paint_dirty(arena: &mut NodeArena, node_id: NodeId) {
    if let Some(element) = arena.get_mut(node_id) {
        element.dirty.insert(DirtyFlags::PAINT);
    }
    // Walk up the parent chain, setting SUBTREE_PAINT. Stop early if the
    // flag is already set (the rest of the chain is already propagated).
    let mut current = arena.get(node_id).map(|e| e.parent).unwrap_or(NodeId::DANGLING);
    while !current.is_dangling() {
        let already_set =
            arena.get(current).map(|e| e.dirty.contains(DirtyFlags::SUBTREE_PAINT)).unwrap_or(true);
        if let Some(elem) = arena.get_mut(current) {
            elem.dirty.insert(DirtyFlags::SUBTREE_PAINT);
        }
        if already_set {
            break;
        }
        current = arena.get(current).map(|e| e.parent).unwrap_or(NodeId::DANGLING);
    }
}

/// Recursively set the PAINT and SUBTREE_PAINT dirty flags on every node in
/// the subtree. Used after the initial tree build so the first frame renders
/// all elements.
pub fn mark_paint_dirty(arena: &mut NodeArena, node_id: NodeId) {
    let children = arena.children(node_id);

    if let Some(element) = arena.get_mut(node_id) {
        element.dirty |= DirtyFlags::PAINT | DirtyFlags::SUBTREE_PAINT;
    }

    for child_id in children {
        mark_paint_dirty(arena, child_id);
    }
}

/// Recursively set the LAYOUT dirty flag on every node in the subtree.
pub fn mark_layout_dirty(arena: &mut NodeArena, node_id: NodeId) {
    let children = arena.children(node_id);

    if let Some(element) = arena.get_mut(node_id) {
        element.dirty |= DirtyFlags::LAYOUT;
    }

    for child_id in children {
        mark_layout_dirty(arena, child_id);
    }
}

/// Full layout pipeline: sync elements to taffy, compute layout, read results back, clear flags,
/// and dispatch resize callbacks for elements whose dimensions changed.
pub fn run_layout_pipeline(
    arena: &mut NodeArena,
    taffy: &mut taffy::TaffyTree<TextMeasureCtx>,
    root: NodeId,
    font_system: &mut FontSystem,
    width: f32,
    height: f32,
    cache: &mut TextMeasureCache,
) {
    layout::sync_element_to_taffy(arena, taffy, root, font_system, width, height);
    if let Some(tn) = arena.get(root).and_then(|e| e.taffy_node) {
        layout::compute_layout(taffy, tn, width, height, font_system, cache);
        layout::read_layout_results(arena, taffy, root, 0.0, 0.0);
    }
    layout::clear_dirty_flags(arena, root);
    dispatch_resize_callbacks(arena, root);
}

/// Epsilon threshold for resize detection (0.5 pixels).
const RESIZE_EPSILON: f32 = 0.5;

/// Walk the element tree after layout, detect dimension changes, and fire
/// `on_resize` callbacks in batch. Updates `prev_width`/`prev_height` after
/// dispatching so that the next frame can detect further changes.
pub fn dispatch_resize_callbacks(arena: &mut NodeArena, root: NodeId) -> bool {
    // Phase 1: collect (NodeId, callback, new_width, new_height) for elements that resized
    let mut pending: Vec<(NodeId, std::sync::Arc<dyn Fn(f32, f32) + Send + Sync>, f32, f32)> =
        Vec::new();
    collect_resized_elements(arena, root, &mut pending);
    let fired = !pending.is_empty();

    // Phase 2: dispatch all callbacks
    for (node_id, callback, width, height) in &pending {
        callback(*width, *height);
        // Update prev dimensions so subsequent frames only fire on new changes
        if let Some(element) = arena.get_mut(*node_id) {
            element.prev_width = *width;
            element.prev_height = *height;
        }
    }

    fired
}

fn collect_resized_elements(
    arena: &NodeArena,
    node_id: NodeId,
    pending: &mut Vec<(NodeId, std::sync::Arc<dyn Fn(f32, f32) + Send + Sync>, f32, f32)>,
) {
    if let Some(element) = arena.get(node_id) {
        let rect = element.layout_rect;
        let w_changed = (rect.width - element.prev_width).abs() > RESIZE_EPSILON;
        let h_changed = (rect.height - element.prev_height).abs() > RESIZE_EPSILON;

        if (w_changed || h_changed) && element.on_resize.is_some() {
            let cb = element.on_resize.clone().unwrap();
            pending.push((node_id, cb, rect.width, rect.height));
        }

        let mut child = element.first_child;
        while !child.is_dangling() {
            collect_resized_elements(arena, child, pending);
            child = arena.get(child).map(|e| e.next_sibling).unwrap_or(NodeId::DANGLING);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{ElementDef, Tag};
    use crate::style::parse::CompiledStylesheet;
    use crate::style::types::Background;
    use std::time::Duration;

    /// Build a minimal arena with one element, resolve styles with a stylesheet,
    /// and return (arena, root, stylesheet).
    fn setup(css: &str) -> (NodeArena, NodeId, CompiledStylesheet) {
        let stylesheet = CompiledStylesheet::parse(css);
        let mut arena = NodeArena::new();
        let mut taffy = taffy::TaffyTree::<TextMeasureCtx>::new();

        let def = ElementDef::new(Tag::Div).with_class("box");
        let root = build_tree_from_def(&def, &mut arena, &mut taffy, NodeId::DANGLING);

        // Initial style resolve (no transitions yet since no previous_style).
        resolve_all_styles(&mut arena, &stylesheet, root, NodeId::DANGLING, None, NodeId::DANGLING);

        (arena, root, stylesheet)
    }

    #[test]
    fn test_transition_starts_on_hover() {
        let css = r#"
            .box {
                opacity: 1.0;
                transition: opacity 0.5s linear;
            }
            .box:hover {
                opacity: 0.5;
            }
        "#;

        let (mut arena, root, stylesheet) = setup(css);
        let now = Instant::now();
        let mut at = ActiveTransitions::default();

        // Verify initial opacity.
        assert!((arena.get(root).unwrap().computed_style.opacity - 1.0).abs() < 1e-6);

        // Simulate hover: resolve styles with root as hovered.
        resolve_all_styles_with_transitions(
            &mut arena,
            &stylesheet,
            root,
            root, // hovered
            None,
            NodeId::DANGLING,
            false,
            Some(now),
            Some(&mut at),
        );

        // The element should now have a running transition.
        let element = arena.get(root).unwrap();
        assert_eq!(element.running_transitions.len(), 1);
        assert!(at.has_active());

        // The target style should be 0.5 (hover), but the computed style should
        // still be at the start value since we haven't ticked yet.
        // (resolve_all_styles sets the target style directly, transitions override on tick)
    }

    #[test]
    fn test_transition_tick_interpolates() {
        let css = r#"
            .box {
                opacity: 1.0;
                transition: opacity 1s linear;
            }
            .box:hover {
                opacity: 0.0;
            }
        "#;

        let (mut arena, root, stylesheet) = setup(css);
        let now = Instant::now();
        let mut at = ActiveTransitions::default();

        // Hover triggers transition.
        resolve_all_styles_with_transitions(
            &mut arena,
            &stylesheet,
            root,
            root,
            None,
            NodeId::DANGLING,
            false,
            Some(now),
            Some(&mut at),
        );

        assert!(at.has_active());

        // Tick at 500ms: opacity should be ~0.5.
        let mid = now + Duration::from_millis(500);
        tick_all_transitions(&mut arena, &mut at, mid);

        let opacity = arena.get(root).unwrap().computed_style.opacity;
        assert!((opacity - 0.5).abs() < 0.1, "opacity at 500ms should be ~0.5, got {}", opacity);

        // Tick at 1000ms: transition complete, opacity should be 0.0.
        let end = now + Duration::from_millis(1001);
        let still_active = tick_all_transitions(&mut arena, &mut at, end);
        assert!(!still_active, "transition should be complete");

        let opacity = arena.get(root).unwrap().computed_style.opacity;
        assert!((opacity - 0.0).abs() < 1e-6, "opacity at 1000ms should be 0.0, got {}", opacity);
    }

    #[test]
    fn test_transition_color_in_oklab() {
        let css = r#"
            .box {
                background: #ff0000;
                transition: background 1s linear;
            }
            .box:hover {
                background: #0000ff;
            }
        "#;

        let (mut arena, root, stylesheet) = setup(css);
        let now = Instant::now();
        let mut at = ActiveTransitions::default();

        // Hover.
        resolve_all_styles_with_transitions(
            &mut arena,
            &stylesheet,
            root,
            root,
            None,
            NodeId::DANGLING,
            false,
            Some(now),
            Some(&mut at),
        );

        // Tick at midpoint.
        let mid = now + Duration::from_millis(500);
        tick_all_transitions(&mut arena, &mut at, mid);

        let bg = arena.get(root).unwrap().computed_style.background.clone();
        if let Background::Color(c) = bg {
            // In Oklab space, the midpoint of red and blue should not be dark gray.
            // It should have reasonable brightness.
            let brightness = c.r as f32 * 0.299 + c.g as f32 * 0.587 + c.b as f32 * 0.114;
            assert!(
                brightness > 30.0,
                "Oklab midpoint should not be too dark; brightness = {}, color = {:?}",
                brightness,
                c
            );
        } else {
            panic!("expected Background::Color");
        }
    }

    #[test]
    fn test_no_transition_without_property() {
        // Elements without a transition property should not get transitions.
        let css = r#"
            .box { opacity: 1.0; }
            .box:hover { opacity: 0.5; }
        "#;

        let (mut arena, root, stylesheet) = setup(css);
        let now = Instant::now();
        let mut at = ActiveTransitions::default();

        resolve_all_styles_with_transitions(
            &mut arena,
            &stylesheet,
            root,
            root,
            None,
            NodeId::DANGLING,
            false,
            Some(now),
            Some(&mut at),
        );

        // No transitions should be created because no `transition` CSS property.
        let element = arena.get(root).unwrap();
        assert!(element.running_transitions.is_empty());
        assert!(!at.has_active());
    }

    #[test]
    fn test_transition_all_property() {
        let css = r#"
            .box {
                opacity: 1.0;
                gap: 0;
                transition: all 0.5s ease;
            }
            .box:hover {
                opacity: 0.5;
                gap: 10px;
            }
        "#;

        let (mut arena, root, stylesheet) = setup(css);
        let now = Instant::now();
        let mut at = ActiveTransitions::default();

        resolve_all_styles_with_transitions(
            &mut arena,
            &stylesheet,
            root,
            root,
            None,
            NodeId::DANGLING,
            false,
            Some(now),
            Some(&mut at),
        );

        // Should create transitions for both changed properties.
        let element = arena.get(root).unwrap();
        assert_eq!(
            element.running_transitions.len(),
            2,
            "expected 2 transitions (opacity + gap), got {}",
            element.running_transitions.len()
        );
    }

    #[test]
    fn test_active_transitions_cleanup() {
        let css = r#"
            .box {
                opacity: 1.0;
                transition: opacity 0.1s linear;
            }
            .box:hover {
                opacity: 0.0;
            }
        "#;

        let (mut arena, root, stylesheet) = setup(css);
        let now = Instant::now();
        let mut at = ActiveTransitions::default();

        resolve_all_styles_with_transitions(
            &mut arena,
            &stylesheet,
            root,
            root,
            None,
            NodeId::DANGLING,
            false,
            Some(now),
            Some(&mut at),
        );

        assert!(at.has_active());

        // Tick past completion.
        let after = now + Duration::from_millis(200);
        let still_active = tick_all_transitions(&mut arena, &mut at, after);

        assert!(!still_active);
        assert!(!at.has_active());
        assert!(arena.get(root).unwrap().running_transitions.is_empty());
    }

    // ------------------------------------------------------------------
    // @keyframes + animation: end to end tests (issue #129)
    // ------------------------------------------------------------------

    /// Helper: set up a single element tree with the provided stylesheet and
    /// return (arena, root, stylesheet, driver).
    fn setup_animation(css: &str) -> (NodeArena, NodeId, CompiledStylesheet, AnimationDriver) {
        let stylesheet = CompiledStylesheet::parse(css);
        let mut arena = NodeArena::new();
        let mut taffy = taffy::TaffyTree::<TextMeasureCtx>::new();
        let def = ElementDef::new(Tag::Div).with_class("box");
        let root = build_tree_from_def(&def, &mut arena, &mut taffy, NodeId::DANGLING);
        resolve_all_styles(&mut arena, &stylesheet, root, NodeId::DANGLING, None, NodeId::DANGLING);
        (arena, root, stylesheet, AnimationDriver::new())
    }

    #[test]
    fn test_resolver_creates_animation_state() {
        let css = ".box { animation: fade 1s linear; } \
                   @keyframes fade { from { opacity: 0; } to { opacity: 1; } }";
        let (arena, root, _sheet, mut driver) = setup_animation(css);
        let now = Instant::now();
        sync_all_animations(&arena, &mut driver, root, now);
        assert!(driver.running.contains_key(&root));
        assert_eq!(driver.running[&root].len(), 1);
    }

    #[test]
    fn test_resolver_replaces_state_on_name_change() {
        let css = ".box { animation: first 1s linear; } \
                   @keyframes first { from { opacity: 0; } to { opacity: 1; } } \
                   @keyframes second { from { opacity: 0; } to { opacity: 0.5; } }";
        let (mut arena, root, stylesheet, mut driver) = setup_animation(css);
        let now = Instant::now();
        sync_all_animations(&arena, &mut driver, root, now);
        assert_eq!(driver.running[&root][0].name.as_ref(), "first");

        // Rewrite the element's animation list directly to simulate a style
        // change and resync the driver.
        {
            let el = arena.get_mut(root).unwrap();
            el.computed_style.animations.clear();
            el.computed_style.animations.push(crate::style::types::AnimationDef {
                name: Some(std::sync::Arc::<str>::from("second")),
                duration: Duration::from_secs(1),
                timing_function: transition::TimingFunction::Linear,
                delay: Duration::ZERO,
                delay_nanos: 0,
                iteration_count: crate::style::types::IterationCount::Finite(1.0),
                direction: crate::style::types::AnimationDirection::Normal,
                fill_mode: crate::style::types::AnimationFillMode::None,
                play_state: crate::style::types::AnimationPlayState::Running,
            });
        }
        sync_all_animations(&arena, &mut driver, root, now);
        assert_eq!(driver.running[&root][0].name.as_ref(), "second");
        let _ = stylesheet; // silence unused
    }

    #[test]
    fn test_resolver_clears_state_when_name_removed() {
        let css = ".box { animation: fade 1s linear; } \
                   @keyframes fade { from { opacity: 0; } to { opacity: 1; } }";
        let (mut arena, root, _sheet, mut driver) = setup_animation(css);
        sync_all_animations(&arena, &mut driver, root, Instant::now());
        assert!(driver.running.contains_key(&root));

        // Remove the animation from the element's cascaded style.
        arena.get_mut(root).unwrap().computed_style.animations.clear();
        sync_all_animations(&arena, &mut driver, root, Instant::now());
        assert!(!driver.running.contains_key(&root));
    }

    #[test]
    fn test_pulse_dot_at_quarter_second() {
        // Terminal manager pulse-dot: opacity 1 at 0% and 100%, 0.4 at 50%.
        // Duration 2s, ease-in-out. Sample opacity at known offsets and
        // confirm the curve is monotonic on each half.
        let css = "@keyframes pulse-dot { \
                       0%, 100% { opacity: 1; } \
                       50% { opacity: 0.4; } \
                   } \
                   .box { animation: pulse-dot 2s ease-in-out infinite; }";
        let (mut arena, root, stylesheet, mut driver) = setup_animation(css);
        let now = Instant::now();
        sync_all_animations(&arena, &mut driver, root, now);

        // t = 0: opacity starts at 1.
        driver.tick(&mut arena, &stylesheet, now);
        let t0 = arena.get(root).unwrap().computed_style.opacity;
        assert!((t0 - 1.0).abs() < 0.05, "expected ~1.0 at 0ms, got {}", t0);

        // t = 1000ms: opacity is at the 50% mark which is 0.4.
        driver.tick(&mut arena, &stylesheet, now + Duration::from_millis(1000));
        let t_mid = arena.get(root).unwrap().computed_style.opacity;
        assert!((t_mid - 0.4).abs() < 0.05, "expected ~0.4 at 1000ms, got {}", t_mid);

        // t = 2000ms: back at 1 (second iteration boundary).
        driver.tick(&mut arena, &stylesheet, now + Duration::from_millis(2000));
        let t2 = arena.get(root).unwrap().computed_style.opacity;
        assert!((t2 - 1.0).abs() < 0.05, "expected ~1.0 at 2000ms, got {}", t2);
    }

    #[test]
    fn test_modal_fade_in_full_cycle() {
        // Terminal manager fade-in: opacity rises from 0 to 1 over 200ms.
        let css = "@keyframes fade-in { from { opacity: 0; } to { opacity: 1; } } \
                   .box { \
                     opacity: 1; \
                     animation: fade-in 200ms cubic-bezier(0.22, 0.61, 0.36, 1); \
                   }";
        let (mut arena, root, stylesheet, mut driver) = setup_animation(css);
        let now = Instant::now();
        sync_all_animations(&arena, &mut driver, root, now);

        let mut samples = Vec::new();
        for step in 0..=4 {
            let t = now + Duration::from_millis(step * 50);
            driver.tick(&mut arena, &stylesheet, t);
            samples.push(arena.get(root).unwrap().computed_style.opacity);
        }
        assert!(samples[0] < 0.05);
        assert!(samples[4] > 0.95);
        for i in 1..samples.len() {
            assert!(
                samples[i] >= samples[i - 1] - 1e-4,
                "fade-in must be monotonic; saw {} -> {}",
                samples[i - 1],
                samples[i]
            );
        }
    }

    /// Regression: mark_node_paint_dirty must set PAINT on the target
    /// node and propagate SUBTREE_PAINT up the ancestor chain.
    #[test]
    fn mark_node_paint_dirty_propagates_subtree_paint() {
        let mut arena = NodeArena::new();
        let root = arena.alloc(crate::element::Element::new(Tag::Div));
        let mid = arena.alloc(crate::element::Element::new(Tag::Div));
        let leaf = arena.alloc(crate::element::Element::new(Tag::Div));
        arena.append_child(root, mid);
        arena.append_child(mid, leaf);

        // Start clean.
        for &nid in &[root, mid, leaf] {
            arena.get_mut(nid).unwrap().dirty = DirtyFlags::empty();
        }

        mark_node_paint_dirty(&mut arena, leaf);

        // The leaf should have PAINT.
        assert!(arena.get(leaf).unwrap().dirty.contains(DirtyFlags::PAINT));

        // The parent and grandparent should have SUBTREE_PAINT.
        assert!(
            arena.get(mid).unwrap().dirty.contains(DirtyFlags::SUBTREE_PAINT),
            "parent should carry SUBTREE_PAINT"
        );
        assert!(
            arena.get(root).unwrap().dirty.contains(DirtyFlags::SUBTREE_PAINT),
            "grandparent should carry SUBTREE_PAINT"
        );

        // Parent/grandparent should NOT have PAINT on themselves.
        assert!(
            !arena.get(mid).unwrap().dirty.contains(DirtyFlags::PAINT),
            "parent should not have PAINT"
        );
        assert!(
            !arena.get(root).unwrap().dirty.contains(DirtyFlags::PAINT),
            "grandparent should not have PAINT"
        );
    }

    /// Regression: tick_all_transitions must set PAINT on ticked nodes
    /// so the batch builder re-renders them with interpolated values.
    #[test]
    fn tick_transitions_sets_paint_dirty() {
        let css = ".box { background: #000; transition: background 500ms linear; }
                   .box:hover { background: #fff; }";
        let stylesheet = CompiledStylesheet::parse(css);
        let mut arena = NodeArena::new();
        let mut taffy = taffy::TaffyTree::<TextMeasureCtx>::new();

        let def = ElementDef::new(Tag::Div).with_class("box");
        let root = build_tree_from_def(&def, &mut arena, &mut taffy, NodeId::DANGLING);

        let now = std::time::Instant::now();

        // Initial resolve (no hover, establishes previous_style).
        resolve_all_styles_with_transitions(
            &mut arena,
            &stylesheet,
            root,
            NodeId::DANGLING,
            None,
            NodeId::DANGLING,
            false,
            Some(now),
            None,
        );

        // Hover resolve to start the transition.
        let mut active = ActiveTransitions::default();
        resolve_all_styles_with_transitions(
            &mut arena,
            &stylesheet,
            root,
            root, // hover on root
            None,
            NodeId::DANGLING,
            false,
            Some(now),
            Some(&mut active),
        );

        // Clear all flags to simulate what clear_dirty_flags does.
        arena.get_mut(root).unwrap().dirty = DirtyFlags::empty();

        // Tick transitions.
        let mid = now + Duration::from_millis(250);
        tick_all_transitions(&mut arena, &mut active, mid);

        // The ticked node must be paint-dirty.
        assert!(
            arena.get(root).unwrap().dirty.contains(DirtyFlags::PAINT),
            "ticked transition node must be PAINT dirty; got {:?}",
            arena.get(root).unwrap().dirty
        );
    }
}
