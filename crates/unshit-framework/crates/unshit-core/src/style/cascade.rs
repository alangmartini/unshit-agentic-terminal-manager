use crate::element::Element;
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
    resolve_style_with_pseudo(arena, stylesheet, node_id, hovered, active, focused, false, None)
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
    )
}

/// Resolve styles for `node_id`. When `pseudo_target` is `Some(pe)`, this
/// pass only applies rules whose selector chain terminates in that specific
/// pseudo element, and those rules are matched against the host element.
/// When `None`, pseudo element rules are skipped so they never leak onto the
/// host's computed style.
pub fn resolve_style_with_pseudo(
    arena: &NodeArena,
    stylesheet: &CompiledStylesheet,
    node_id: NodeId,
    hovered: NodeId,
    active: Option<NodeId>,
    focused: NodeId,
    focus_via_keyboard: bool,
    pseudo_target: Option<PseudoElement>,
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
                apply_declaration(&mut style, decl);
            }
        }
    }

    style
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
