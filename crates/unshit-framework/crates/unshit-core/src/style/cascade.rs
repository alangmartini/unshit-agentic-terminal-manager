use crate::element::{Element, Tag};
use crate::event::is_or_ancestor_of;
use crate::id::NodeId;
use crate::style::parse::*;
use crate::style::types::*;
use crate::tree::NodeArena;

pub fn resolve_style(
    arena: &NodeArena,
    stylesheet: &CompiledStylesheet,
    node_id: NodeId,
    hovered: NodeId,
    active: Option<NodeId>,
    focused: NodeId,
) -> ComputedStyle {
    let active_root_scope = active_root_scope_for(arena, stylesheet, node_id);
    resolve_style_with_pseudo(
        arena,
        stylesheet,
        node_id,
        hovered,
        active,
        focused,
        false,
        None,
        active_root_scope,
    )
}

/// Variant that passes through `focus_via_keyboard` for `:focus-visible`.
pub fn resolve_style_fv(
    arena: &NodeArena,
    stylesheet: &CompiledStylesheet,
    node_id: NodeId,
    hovered: NodeId,
    active: Option<NodeId>,
    focused: NodeId,
    focus_via_keyboard: bool,
    active_root_scope: Option<ScopeKey>,
) -> ComputedStyle {
    resolve_style_with_pseudo(
        arena,
        stylesheet,
        node_id,
        hovered,
        active,
        focused,
        focus_via_keyboard,
        None,
        active_root_scope,
    )
}

/// Determine the active root token scope for a cascade rooted anywhere in the
/// tree: walk from `node_id` to the true document root (the node with a dangling
/// parent), then return the first non-base [`TokenScope`] whose selector matches
/// that root via the same `selector_matches` path the rule cascade uses (the
/// `.app.theme-*` scope whose classes the root carries).
///
/// This is computed ONCE per cascade pass and threaded down, so it stays correct
/// even when a hover/focus restyle narrows the cascade to a non-root subtree
/// (the active theme is a property of the document root, not the subtree root).
/// O(depth) once per pass; O(1) per element thereafter.
pub fn active_root_scope_for(
    arena: &NodeArena,
    stylesheet: &CompiledStylesheet,
    node_id: NodeId,
) -> Option<ScopeKey> {
    let scopes = &stylesheet.token_scopes;
    // No non-base scopes => no theme to activate; skip the walk entirely.
    if scopes.non_base().is_empty() {
        return None;
    }

    // Walk to the true document root.
    let mut root_id = node_id;
    while let Some(el) = arena.get(root_id) {
        if el.parent.is_dangling() {
            break;
        }
        root_id = el.parent;
    }
    let root = arena.get(root_id)?;

    // First non-base scope whose selector matches the root wins (source order).
    for scope in scopes.non_base() {
        let Some(chain) = scope.selector.as_ref() else {
            continue;
        };
        if selector_matches(
            chain,
            root,
            arena,
            root_id,
            NodeId::DANGLING,
            None,
            NodeId::DANGLING,
            false,
        ) {
            return Some(scope.key);
        }
    }
    None
}

/// Resolve styles for `node_id`. When `pseudo_target` is `Some(pe)`, this
/// pass only applies rules whose selector chain terminates in that specific
/// pseudo element, and those rules are matched against the host element.
/// When `None`, pseudo element rules are skipped so they never leak onto the
/// host's computed style.
#[allow(clippy::too_many_arguments)]
pub fn resolve_style_with_pseudo(
    arena: &NodeArena,
    stylesheet: &CompiledStylesheet,
    node_id: NodeId,
    hovered: NodeId,
    active: Option<NodeId>,
    focused: NodeId,
    focus_via_keyboard: bool,
    pseudo_target: Option<PseudoElement>,
    active_root_scope: Option<ScopeKey>,
) -> ComputedStyle {
    let Some(element) = arena.get(node_id) else {
        return ComputedStyle::default();
    };

    let mut style = ComputedStyle::default();

    if !element.parent.is_dangling() {
        if let Some(parent) = arena.get(element.parent) {
            style.inherit_from(&parent.computed_style);
        }
    }

    apply_user_agent_defaults(&mut style, element.tag);

    // Build this element's ordered token-resolution environment ONCE, before the
    // rule loop, so every `Deferred` declaration that matches resolves its
    // `var()` against the same specificity-ordered scopes:
    //   [ self widget scope (if the element carries one),
    //     active root theme scope (a property of the document root),
    //     :root base ].
    // Almost always this is exactly `[:root]` or `[:root, theme]`; the self slot
    // only appears for the handful of widget elements (e.g. `.theme-chip.<name>`)
    // that carry their own token class. The self-scope match is cheap (a few
    // class comparisons against the element) and is skipped entirely when the
    // stylesheet declares no non-base scopes.
    let self_scope_vars = self_scope_vars_for(
        stylesheet,
        element,
        arena,
        node_id,
        hovered,
        active,
        focused,
        focus_via_keyboard,
        active_root_scope,
    );
    let has_self_scope = self_scope_vars.is_some();
    let env = ScopeEnv::new(
        self_scope_vars,
        active_root_scope.and_then(|k| stylesheet.token_scopes.vars_for(k)),
        stylesheet.token_scopes.base_vars(),
    );
    // Stable-within-a-parse identity for the deferred memo: a process-unique id
    // assigned at parse time. It changes on re-parse / hot reload (and is immune
    // to allocator address reuse), so the memo self-invalidates and never serves
    // a stale entry for a different stylesheet.
    let stylesheet_id = stylesheet.parse_id;
    // Deferred declarations that fail to resolve+re-parse are routed here so the
    // gap stays visible (mirrors the parse-time `dropped` sink). The cascade has
    // no shared sink, so this is per-element and discarded; a malformed scoped
    // var() still routes to `dropped` at parse time when it is a custom-property
    // definition, and the live path here drops it rather than silently applying.
    let mut deferred_dropped = Vec::new();

    for rule in &stylesheet.rules {
        let rule_pseudo = rule.selector.pseudo_element();
        if rule_pseudo != pseudo_target {
            continue;
        }
        if selector_matches(
            &rule.selector,
            element,
            arena,
            node_id,
            hovered,
            active,
            focused,
            focus_via_keyboard,
        ) {
            for decl in &rule.declarations {
                match decl {
                    StyleDeclaration::Deferred { property, raw_value, scope_hint } => {
                        apply_deferred_against_env_memoized(
                            &mut style,
                            property,
                            raw_value,
                            *scope_hint,
                            &env,
                            &mut deferred_dropped,
                            stylesheet_id,
                            active_root_scope,
                            has_self_scope,
                        );
                    }
                    _ => apply_declaration(&mut style, decl),
                }
            }
        }
    }

    style
}

/// Find the widget self-scope var map for `element`: the highest-specificity
/// non-base [`TokenScope`] whose selector matches the element ITSELF (e.g.
/// `.theme-chip.dracula` on a theme chip). The active root theme scope is
/// excluded here — it is matched against the document root, not the element, and
/// is supplied separately to the env. Returns `None` for the overwhelmingly
/// common element that carries no widget token class.
#[allow(clippy::too_many_arguments)]
fn self_scope_vars_for<'a>(
    stylesheet: &'a CompiledStylesheet,
    element: &Element,
    arena: &NodeArena,
    node_id: NodeId,
    hovered: NodeId,
    active: Option<NodeId>,
    focused: NodeId,
    focus_via_keyboard: bool,
    active_root_scope: Option<ScopeKey>,
) -> Option<&'a std::collections::HashMap<String, String>> {
    let scopes = &stylesheet.token_scopes;
    let non_base = scopes.non_base();
    if non_base.is_empty() {
        return None;
    }
    // Perf gate: a widget self scope can only match this element if the element
    // carries one of the classes that some non-base scope selector keys on. The
    // union of those classes is precomputed once at parse time, so this is a few
    // cheap hash lookups. The overwhelmingly common element shares none of them,
    // letting us skip the `O(non_base scopes)` `selector_matches` walk entirely.
    if !scopes.element_may_have_self_scope(&element.classes) {
        return None;
    }
    // Highest specificity first so a more-specific widget scope wins; ties break
    // on later source order (the cascade's tiebreak).
    let mut best: Option<&TokenScope> = None;
    for scope in non_base {
        // The active root scope is handled separately (matched on the root).
        if Some(scope.key) == active_root_scope {
            continue;
        }
        let Some(chain) = scope.selector.as_ref() else {
            continue;
        };
        if selector_matches(
            chain,
            element,
            arena,
            node_id,
            hovered,
            active,
            focused,
            focus_via_keyboard,
        ) {
            let wins = match best {
                None => true,
                Some(b) => {
                    (scope.specificity, scope.source_order) >= (b.specificity, b.source_order)
                }
            };
            if wins {
                best = Some(scope);
            }
        }
    }
    best.map(|s| s.vars.as_ref())
}

fn apply_user_agent_defaults(style: &mut ComputedStyle, tag: Tag) {
    if tag == Tag::Button {
        style.text_align = TextAlign::Center;
    }
}

pub fn resolve_selection_style(
    arena: &NodeArena,
    stylesheet: &CompiledStylesheet,
    node_id: NodeId,
    hovered: NodeId,
    active: Option<NodeId>,
    focused: NodeId,
) -> Option<SelectionStyle> {
    let element = arena.get(node_id)?;
    let mut color: Option<Color> = None;
    let mut bg: Option<Color> = None;
    let mut matched = false;
    for rule in &stylesheet.rules {
        if rule.selector.pseudo_element() != Some(PseudoElement::Selection) {
            continue;
        }
        if selector_matches(
            &rule.selector,
            element,
            arena,
            node_id,
            hovered,
            active,
            focused,
            false,
        ) {
            matched = true;
            for decl in &rule.declarations {
                match decl {
                    StyleDeclaration::Color(c) => color = Some(*c),
                    StyleDeclaration::Background(Background::Color(c)) => bg = Some(*c),
                    _ => {}
                }
            }
        }
    }
    if matched {
        Some(SelectionStyle { color, background_color: bg })
    } else {
        None
    }
}

fn selector_matches(
    chain: &SelectorChain,
    element: &Element,
    arena: &NodeArena,
    node_id: NodeId,
    hovered: NodeId,
    active: Option<NodeId>,
    focused: NodeId,
    focus_via_keyboard: bool,
) -> bool {
    if chain.parts.is_empty() {
        return false;
    }

    let (ref last_parts, _) = chain.parts[chain.parts.len() - 1];
    if !parts_match(
        last_parts,
        element,
        arena,
        node_id,
        hovered,
        active,
        focused,
        focus_via_keyboard,
    ) {
        return false;
    }

    if chain.parts.len() == 1 {
        return true;
    }

    let mut current = element.parent;
    let mut part_idx = chain.parts.len() as i32 - 2;

    while part_idx >= 0 && !current.is_dangling() {
        let (ref parts, ref _combinator) = chain.parts[part_idx as usize];
        if let Some(ancestor) = arena.get(current) {
            if parts_match(
                parts,
                ancestor,
                arena,
                current,
                hovered,
                active,
                focused,
                focus_via_keyboard,
            ) {
                part_idx -= 1;
                current = ancestor.parent;
            } else {
                // For child combinator, must be direct parent
                let prev_combinator = if part_idx < chain.parts.len() as i32 - 1 {
                    &chain.parts[(part_idx + 1) as usize].1
                } else {
                    &None
                };
                if matches!(prev_combinator, Some(SelectorCombinator::Child)) {
                    return false;
                }
                current = ancestor.parent;
            }
        } else {
            break;
        }
    }

    part_idx < 0
}

fn parts_match(
    parts: &[SelectorPart],
    element: &Element,
    arena: &NodeArena,
    node_id: NodeId,
    hovered: NodeId,
    active: Option<NodeId>,
    focused: NodeId,
    focus_via_keyboard: bool,
) -> bool {
    for part in parts {
        match part {
            SelectorPart::Universal => {}
            SelectorPart::Tag(tag) => {
                if element.tag_name() != tag.as_str() {
                    return false;
                }
            }
            SelectorPart::Class(cls) => {
                if !element.classes.iter().any(|c| c == cls) {
                    return false;
                }
            }
            SelectorPart::Id(id) => {
                if element.id.as_deref() != Some(id.as_str()) {
                    return false;
                }
            }
            SelectorPart::PseudoElement(_) => {
                // The resolver filters by pseudo element before reaching here,
                // so the tail pseudo part is a pass through against the host.
            }
            SelectorPart::PseudoClass(pseudo) => match pseudo {
                PseudoClass::Hover => {
                    if !is_or_ancestor_of(arena, node_id, hovered) {
                        return false;
                    }
                }
                PseudoClass::Active => {
                    if let Some(active_id) = active {
                        if !is_or_ancestor_of(arena, node_id, active_id) {
                            return false;
                        }
                    } else {
                        return false;
                    }
                }
                PseudoClass::Focus => {
                    // :focus only matches the exact focused element, not ancestors
                    if node_id != focused {
                        return false;
                    }
                }
                PseudoClass::FocusVisible => {
                    // :focus-visible matches the focused element only when focus
                    // was gained via keyboard (Tab), not mouse click.
                    if node_id != focused || !focus_via_keyboard {
                        return false;
                    }
                }
                PseudoClass::FocusWithin => {
                    // :focus-within matches when this node is the focused element
                    // or an ancestor of the focused element.
                    if !is_or_ancestor_of(arena, node_id, focused) {
                        return false;
                    }
                }
                PseudoClass::FirstChild => {
                    if element.prev_sibling != NodeId::DANGLING {
                        return false;
                    }
                }
                PseudoClass::LastChild => {
                    if element.next_sibling != NodeId::DANGLING {
                        return false;
                    }
                }
                PseudoClass::FirstOfType => {
                    let mut current = element.prev_sibling;
                    while !current.is_dangling() {
                        let Some(sibling) = arena.get(current) else {
                            return false;
                        };
                        if sibling.tag == element.tag {
                            return false;
                        }
                        current = sibling.prev_sibling;
                    }
                }
                PseudoClass::LastOfType => {
                    let mut current = element.next_sibling;
                    while !current.is_dangling() {
                        let Some(sibling) = arena.get(current) else {
                            return false;
                        };
                        if sibling.tag == element.tag {
                            return false;
                        }
                        current = sibling.next_sibling;
                    }
                }
                PseudoClass::NthChild(n) => {
                    let parent_id = element.parent;
                    if parent_id.is_dangling() {
                        return false;
                    }
                    let parent = match arena.get(parent_id) {
                        Some(p) => p,
                        None => return false,
                    };
                    let mut current = parent.first_child;
                    let mut position = 1i32;
                    while !current.is_dangling() {
                        if current == node_id {
                            break;
                        }
                        if let Some(sibling) = arena.get(current) {
                            current = sibling.next_sibling;
                            position += 1;
                        } else {
                            return false;
                        }
                    }
                    if current.is_dangling() || position != *n {
                        return false;
                    }
                }
                PseudoClass::Not(inner_part) => {
                    let inner_matches = match inner_part.as_ref() {
                        SelectorPart::Tag(tag) => element.tag_name() == tag.as_str(),
                        SelectorPart::Class(cls) => element.classes.iter().any(|c| c == cls),
                        SelectorPart::Id(id) => element.id.as_deref() == Some(id.as_str()),
                        SelectorPart::PseudoElement(_)
                        | SelectorPart::PseudoClass(_)
                        | SelectorPart::Universal => false,
                    };
                    if inner_matches {
                        return false;
                    }
                }
            },
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{Element, Tag};

    fn element_with_class(tag: Tag, class: &str) -> Element {
        let mut element = Element::new(tag);
        element.classes.push(class.to_string());
        element
    }

    #[test]
    fn last_of_type_matches_only_last_sibling_with_same_tag() {
        let sheet = CompiledStylesheet::parse(
            ".cell { border-right: 1px solid #ffffff; } .cell:last-of-type { border-right: none; }",
        );
        let mut arena = NodeArena::new();
        let root = arena.alloc(Element::new(Tag::Div));
        let first = arena.alloc(element_with_class(Tag::Span, "cell"));
        let marker = arena.alloc(Element::new(Tag::Div));
        let last = arena.alloc(element_with_class(Tag::Span, "cell"));
        arena.append_child(root, first);
        arena.append_child(root, marker);
        arena.append_child(root, last);

        let first_style =
            resolve_style(&arena, &sheet, first, NodeId::DANGLING, None, NodeId::DANGLING);
        let last_style =
            resolve_style(&arena, &sheet, last, NodeId::DANGLING, None, NodeId::DANGLING);

        assert_eq!(first_style.border_width.right, 1.0);
        assert_eq!(last_style.border_width.right, 0.0);
    }

    #[test]
    fn first_of_type_matches_only_first_sibling_with_same_tag() {
        let sheet = CompiledStylesheet::parse(
            ".cell { border-left: 1px solid #ffffff; } .cell:first-of-type { border-left: none; }",
        );
        let mut arena = NodeArena::new();
        let root = arena.alloc(Element::new(Tag::Div));
        let first = arena.alloc(element_with_class(Tag::Span, "cell"));
        let marker = arena.alloc(Element::new(Tag::Div));
        let last = arena.alloc(element_with_class(Tag::Span, "cell"));
        arena.append_child(root, first);
        arena.append_child(root, marker);
        arena.append_child(root, last);

        let first_style =
            resolve_style(&arena, &sheet, first, NodeId::DANGLING, None, NodeId::DANGLING);
        let last_style =
            resolve_style(&arena, &sheet, last, NodeId::DANGLING, None, NodeId::DANGLING);

        assert_eq!(first_style.border_width.left, 0.0);
        assert_eq!(last_style.border_width.left, 1.0);
    }
}
