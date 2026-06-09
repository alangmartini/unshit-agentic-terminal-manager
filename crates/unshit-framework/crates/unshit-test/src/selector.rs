//! Selector parser and matching engine for the test framework.
//!
//! Parses CSS-like selector strings into an AST and matches them against
//! elements in a `NodeArena`. Supports compound selectors, descendant and
//! child combinators, attribute selectors, pseudo-classes, and text content
//! matching helpers.

use unshit_core::element::{Element, ElementContent, InputType, Tag};
use unshit_core::id::NodeId;
use unshit_core::tree::NodeArena;

// ---------------------------------------------------------------------------
// AST
// ---------------------------------------------------------------------------

/// A parsed selector, which is a list of compound selectors joined by
/// combinators. For example `".sidebar > .link"` becomes two compound
/// selectors joined by `Combinator::Child`.
#[derive(Debug, Clone, PartialEq)]
pub struct Selector {
    /// The chain is stored left-to-right: the first entry is the leftmost
    /// compound selector, each subsequent entry is a (combinator, compound)
    /// pair.
    pub head: CompoundSelector,
    pub tail: Vec<(Combinator, CompoundSelector)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combinator {
    /// Whitespace: any descendant
    Descendant,
    /// `>`: direct child
    Child,
}

/// A compound selector is a sequence of simple selectors that all must match
/// the same element. For example `div.active#main` is a compound selector
/// with a tag selector, a class selector, and an id selector.
#[derive(Debug, Clone, PartialEq)]
pub struct CompoundSelector {
    pub parts: Vec<SimpleSelector>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SimpleSelector {
    /// Matches a tag name, e.g. `div`
    Tag(String),
    /// Matches a class, e.g. `.active`
    Class(String),
    /// Matches an id, e.g. `#main`
    Id(String),
    /// Matches an attribute, e.g. `[placeholder="Search"]`
    Attribute { name: String, value: String },
    /// Pseudo-class
    PseudoClass(PseudoClass),
}

#[derive(Debug, Clone, PartialEq)]
pub enum PseudoClass {
    FirstChild,
    LastChild,
    NthChild(u32),
    Checked,
    Focused,
}

/// Text-matching pseudo-functions used by the test framework query helpers.
/// These are not CSS pseudo-classes but test-specific extensions:
/// `text("exact")` and `has_text("substring")`.
#[derive(Debug, Clone, PartialEq)]
pub enum TextMatcher {
    Exact(String),
    Contains(String),
}

/// A fully parsed query: an optional selector combined with an optional text
/// matcher. At least one must be present.
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub selector: Option<Selector>,
    pub text: Option<TextMatcher>,
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse a query string into a `Query`.
///
/// The query string can be:
///   - A CSS selector: `.class`, `div.active`, `.parent > .child`
///   - A text matcher: `text("Click me")`, `has_text("Click")`
///   - A combination separated by whitespace (text matcher at end)
///
/// Returns `Err` with a descriptive message for invalid input.
pub fn parse_query(input: &str) -> Result<Query, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("empty selector".to_string());
    }

    // Check for standalone text matchers
    if input.starts_with("text(") || input.starts_with("has_text(") {
        let text = parse_text_matcher(input)?;
        return Ok(Query { selector: None, text: Some(text) });
    }

    let selector = parse_selector(input)?;
    Ok(Query { selector: Some(selector), text: None })
}

fn parse_text_matcher(input: &str) -> Result<TextMatcher, String> {
    if let Some(rest) = input.strip_prefix("text(") {
        let rest = rest.strip_suffix(')').ok_or("unclosed text() matcher")?;
        let content = strip_quotes(rest)?;
        Ok(TextMatcher::Exact(content.to_string()))
    } else if let Some(rest) = input.strip_prefix("has_text(") {
        let rest = rest.strip_suffix(')').ok_or("unclosed has_text() matcher")?;
        let content = strip_quotes(rest)?;
        Ok(TextMatcher::Contains(content.to_string()))
    } else {
        Err(format!("unknown text matcher: {input}"))
    }
}

fn strip_quotes(s: &str) -> Result<&str, String> {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        Ok(&s[1..s.len() - 1])
    } else {
        Err(format!("text matcher argument must be quoted: {s}"))
    }
}

/// Parse a selector string into a `Selector` AST.
pub fn parse_selector(input: &str) -> Result<Selector, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("empty selector".to_string());
    }

    let tokens = tokenize(input)?;
    build_selector(&tokens)
}

// -- Tokenizer --

#[derive(Debug, Clone, PartialEq)]
enum Token {
    /// A compound selector string (no whitespace, no combinator chars)
    Compound(String),
    /// `>` child combinator
    ChildCombinator,
}

fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();
    let mut buf = String::new();

    while let Some(&ch) = chars.peek() {
        match ch {
            ' ' | '\t' | '\n' | '\r' => {
                if !buf.is_empty() {
                    tokens.push(Token::Compound(buf.clone()));
                    buf.clear();
                }
                // skip whitespace
                while chars.peek().map_or(false, |c| c.is_whitespace()) {
                    chars.next();
                }
            }
            '>' => {
                if !buf.is_empty() {
                    tokens.push(Token::Compound(buf.clone()));
                    buf.clear();
                }
                tokens.push(Token::ChildCombinator);
                chars.next();
                // skip trailing whitespace
                while chars.peek().map_or(false, |c| c.is_whitespace()) {
                    chars.next();
                }
            }
            '[' => {
                // Read until matching ']', including the brackets
                buf.push(ch);
                chars.next();
                while let Some(&c) = chars.peek() {
                    buf.push(c);
                    chars.next();
                    if c == ']' {
                        break;
                    }
                }
            }
            _ => {
                buf.push(ch);
                chars.next();
            }
        }
    }
    if !buf.is_empty() {
        tokens.push(Token::Compound(buf));
    }

    if tokens.is_empty() {
        return Err("empty selector after tokenization".to_string());
    }

    Ok(tokens)
}

fn build_selector(tokens: &[Token]) -> Result<Selector, String> {
    // Tokens alternate between Compound and combinator tokens.
    // Whitespace between two Compounds implies Descendant combinator.
    let mut iter = tokens.iter().peekable();

    let first = match iter.next() {
        Some(Token::Compound(s)) => parse_compound(s)?,
        Some(Token::ChildCombinator) => {
            return Err("selector cannot start with '>'".to_string());
        }
        None => return Err("empty selector".to_string()),
    };

    let mut tail = Vec::new();

    while iter.peek().is_some() {
        // Determine combinator
        let combinator = match iter.peek() {
            Some(Token::ChildCombinator) => {
                iter.next(); // consume `>`
                Combinator::Child
            }
            Some(Token::Compound(_)) => Combinator::Descendant,
            None => break,
        };

        let compound = match iter.next() {
            Some(Token::Compound(s)) => parse_compound(s)?,
            Some(Token::ChildCombinator) => {
                return Err("unexpected consecutive '>' combinators".to_string());
            }
            None => {
                return Err("selector ends with a combinator but no following selector".to_string());
            }
        };

        tail.push((combinator, compound));
    }

    Ok(Selector { head: first, tail })
}

fn parse_compound(input: &str) -> Result<CompoundSelector, String> {
    let mut parts = Vec::new();
    let mut chars = input.chars().peekable();

    while chars.peek().is_some() {
        match chars.peek() {
            Some(&'.') => {
                chars.next(); // consume '.'
                let name = take_ident(&mut chars);
                if name.is_empty() {
                    return Err("empty class name after '.'".to_string());
                }
                parts.push(SimpleSelector::Class(name));
            }
            Some(&'#') => {
                chars.next(); // consume '#'
                let name = take_ident(&mut chars);
                if name.is_empty() {
                    return Err("empty id after '#'".to_string());
                }
                parts.push(SimpleSelector::Id(name));
            }
            Some(&'[') => {
                chars.next(); // consume '['
                let attr = take_until(&mut chars, ']');
                // parse name=value or name="value"
                if let Some(eq_pos) = attr.find('=') {
                    let name = attr[..eq_pos].trim().to_string();
                    let raw_value = attr[eq_pos + 1..].trim();
                    let value = if (raw_value.starts_with('"') && raw_value.ends_with('"'))
                        || (raw_value.starts_with('\'') && raw_value.ends_with('\''))
                    {
                        raw_value[1..raw_value.len() - 1].to_string()
                    } else {
                        raw_value.to_string()
                    };
                    parts.push(SimpleSelector::Attribute { name, value });
                } else {
                    return Err(format!("attribute selector must contain '=': [{attr}]"));
                }
            }
            Some(&':') => {
                chars.next(); // consume ':'
                let name = take_ident(&mut chars);
                match name.as_str() {
                    "first-child" => {
                        parts.push(SimpleSelector::PseudoClass(PseudoClass::FirstChild))
                    }
                    "last-child" => parts.push(SimpleSelector::PseudoClass(PseudoClass::LastChild)),
                    "checked" => parts.push(SimpleSelector::PseudoClass(PseudoClass::Checked)),
                    "focused" => parts.push(SimpleSelector::PseudoClass(PseudoClass::Focused)),
                    "nth-child" => {
                        // expect '(' number ')'
                        if chars.peek() != Some(&'(') {
                            return Err("expected '(' after :nth-child".to_string());
                        }
                        chars.next(); // consume '('
                        let num_str = take_until(&mut chars, ')');
                        let n: u32 = num_str
                            .trim()
                            .parse()
                            .map_err(|_| format!("invalid nth-child index: {num_str}"))?;
                        parts.push(SimpleSelector::PseudoClass(PseudoClass::NthChild(n)));
                    }
                    other => {
                        return Err(format!("unsupported pseudo-class: :{other}"));
                    }
                }
            }
            Some(_) => {
                // Must be a tag name
                let name = take_ident(&mut chars);
                if name.is_empty() {
                    // Unknown character
                    let ch = chars.next().unwrap();
                    return Err(format!("unexpected character in selector: '{ch}'"));
                }
                parts.push(SimpleSelector::Tag(name));
            }
            None => break,
        }
    }

    if parts.is_empty() {
        return Err(format!("empty compound selector: {input}"));
    }

    Ok(CompoundSelector { parts })
}

fn take_ident(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut s = String::new();
    while let Some(&ch) = chars.peek() {
        if ch.is_alphanumeric() || ch == '-' || ch == '_' {
            s.push(ch);
            chars.next();
        } else {
            break;
        }
    }
    s
}

fn take_until(chars: &mut std::iter::Peekable<std::str::Chars>, end: char) -> String {
    let mut s = String::new();
    while let Some(&ch) = chars.peek() {
        chars.next();
        if ch == end {
            break;
        }
        s.push(ch);
    }
    s
}

// ---------------------------------------------------------------------------
// Matching
// ---------------------------------------------------------------------------

/// Match a parsed `Query` against the arena, starting from `root`.
/// Returns all matching `NodeId`s in tree order.
pub fn query_all(arena: &NodeArena, root: NodeId, query: &Query) -> Vec<NodeId> {
    walk_tree(arena, root, |node_id, elem| match (&query.selector, &query.text) {
        (Some(sel), None) => matches_full_selector(arena, node_id, sel),
        (None, Some(text)) => matches_text(elem, text),
        (Some(sel), Some(text)) => {
            matches_full_selector(arena, node_id, sel) && matches_text(elem, text)
        }
        (None, None) => false,
    })
}

/// Match a parsed `Query` against the arena, returning the first match.
pub fn query_first(arena: &NodeArena, root: NodeId, query: &Query) -> Option<NodeId> {
    query_all(arena, root, query).into_iter().next()
}

/// Pre-order walk of the tree rooted at `root`, collecting nodes where
/// `predicate` returns true.
fn walk_tree(
    arena: &NodeArena,
    root: NodeId,
    predicate: impl Fn(NodeId, &Element) -> bool,
) -> Vec<NodeId> {
    let mut results = Vec::new();
    let mut stack = vec![root];

    while let Some(node_id) = stack.pop() {
        if let Some(elem) = arena.get(node_id) {
            // Anonymous text boxes mirror their host's text; matching them
            // would make every text locator on a mixed-content host resolve
            // twice. The host (which keeps its content) is the match.
            if !elem.anonymous && predicate(node_id, elem) {
                results.push(node_id);
            }
            let children = arena.children(node_id);
            for child in children.iter().rev() {
                stack.push(*child);
            }
        }
    }

    results
}

/// Check if a given node matches the full selector chain (head + combinators).
fn matches_full_selector(arena: &NodeArena, node_id: NodeId, selector: &Selector) -> bool {
    let elem = match arena.get(node_id) {
        Some(e) => e,
        None => return false,
    };

    if selector.tail.is_empty() {
        return matches_compound(arena, node_id, elem, &selector.head);
    }

    // The rightmost compound must match the candidate node.
    let last_compound = &selector.tail.last().unwrap().1;

    if !matches_compound(arena, node_id, elem, last_compound) {
        return false;
    }

    // Now walk backwards through the chain
    let chain_len = selector.tail.len();
    let mut current = node_id;

    for i in (0..chain_len).rev() {
        let (combinator, _) = &selector.tail[i];
        let target_compound = if i == 0 { &selector.head } else { &selector.tail[i - 1].1 };

        match combinator {
            Combinator::Child => {
                // Parent must match
                let parent_id = match arena.get(current) {
                    Some(e) => e.parent,
                    None => return false,
                };
                if parent_id.is_dangling() {
                    return false;
                }
                let parent = match arena.get(parent_id) {
                    Some(e) => e,
                    None => return false,
                };
                if !matches_compound(arena, parent_id, parent, target_compound) {
                    return false;
                }
                current = parent_id;
            }
            Combinator::Descendant => {
                // Walk up ancestors until one matches
                let mut ancestor_id = match arena.get(current) {
                    Some(e) => e.parent,
                    None => return false,
                };
                loop {
                    if ancestor_id.is_dangling() {
                        return false;
                    }
                    let ancestor = match arena.get(ancestor_id) {
                        Some(e) => e,
                        None => return false,
                    };
                    if matches_compound(arena, ancestor_id, ancestor, target_compound) {
                        current = ancestor_id;
                        break;
                    }
                    ancestor_id = ancestor.parent;
                }
            }
        }
    }

    true
}

/// Check if an element matches a compound selector (all parts must match).
fn matches_compound(
    arena: &NodeArena,
    node_id: NodeId,
    element: &Element,
    compound: &CompoundSelector,
) -> bool {
    compound.parts.iter().all(|part| matches_simple(arena, node_id, element, part))
}

/// Check if an element matches a single simple selector.
fn matches_simple(
    arena: &NodeArena,
    node_id: NodeId,
    element: &Element,
    selector: &SimpleSelector,
) -> bool {
    match selector {
        SimpleSelector::Tag(tag) => element.tag_name() == tag.as_str(),
        SimpleSelector::Class(cls) => element.classes.iter().any(|c| c == cls),
        SimpleSelector::Id(id) => element.id.as_deref() == Some(id.as_str()),
        SimpleSelector::Attribute { name, value } => match name.as_str() {
            "placeholder" => element.placeholder.as_deref() == Some(value.as_str()),
            "type" => {
                let elem_type = match element.input_state.input_type {
                    InputType::Text => "text",
                    InputType::Password => "password",
                    InputType::Checkbox => "checkbox",
                    InputType::Radio => "radio",
                    InputType::Number => "number",
                    InputType::Range => "range",
                    InputType::Hidden => "hidden",
                };
                elem_type == value.as_str()
            }
            "name" => element.name.as_deref() == Some(value.as_str()),
            "id" => element.id.as_deref() == Some(value.as_str()),
            "class" => element.classes.iter().any(|c| c == value),
            "value" => element.input_state.value == *value,
            _ => false,
        },
        SimpleSelector::PseudoClass(pseudo) => match pseudo {
            PseudoClass::FirstChild => is_first_child(arena, node_id, element),
            PseudoClass::LastChild => is_last_child(arena, node_id, element),
            PseudoClass::NthChild(n) => is_nth_child(arena, node_id, element, *n),
            PseudoClass::Checked => element.tag == Tag::Input && element.input_state.checked,
            // Focus lives on InteractionState, not on elements; always false here.
            PseudoClass::Focused => false,
        },
    }
}

/// Return the 1-based child index of `node_id` among its non-synthetic
/// siblings, plus the total count. Returns `None` when the node has no parent.
fn child_position(arena: &NodeArena, node_id: NodeId, element: &Element) -> Option<(u32, u32)> {
    let parent_id = element.parent;
    if parent_id.is_dangling() {
        return None;
    }
    let parent = arena.get(parent_id)?;
    let mut child = parent.first_child;
    let mut index = 1u32;
    let mut found_index = None;
    let mut total = 0u32;

    while !child.is_dangling() {
        let c = arena.get(child)?;
        if !c.synthetic {
            if child == node_id {
                found_index = Some(index);
            }
            total += 1;
            index += 1;
        }
        child = c.next_sibling;
    }

    found_index.map(|i| (i, total))
}

fn is_first_child(arena: &NodeArena, node_id: NodeId, element: &Element) -> bool {
    child_position(arena, node_id, element).map_or(false, |(i, _)| i == 1)
}

fn is_last_child(arena: &NodeArena, node_id: NodeId, element: &Element) -> bool {
    child_position(arena, node_id, element).map_or(false, |(i, total)| i == total)
}

fn is_nth_child(arena: &NodeArena, node_id: NodeId, element: &Element, n: u32) -> bool {
    child_position(arena, node_id, element).map_or(false, |(i, _)| i == n)
}

fn matches_text(element: &Element, text: &TextMatcher) -> bool {
    let content_str = match &element.content {
        ElementContent::Text(s) => s.as_str(),
        _ => return false,
    };
    match text {
        TextMatcher::Exact(expected) => content_str == expected,
        TextMatcher::Contains(sub) => content_str.contains(sub.as_str()),
    }
}

// ---------------------------------------------------------------------------
// Legacy compatibility
// ---------------------------------------------------------------------------

/// Simple single-token selector matching (`.class`, `#id`, `tag`).
/// Used by `WindowedTest` which iterates the arena directly.
pub fn matches_simple_selector(selector: &str, element: &Element) -> bool {
    if let Some(class) = selector.strip_prefix('.') {
        element.classes.iter().any(|c| c == class)
    } else if let Some(id) = selector.strip_prefix('#') {
        element.id.as_deref() == Some(id)
    } else {
        element.tag_name() == selector
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_class() {
        let q = parse_query(".active").unwrap();
        let sel = q.selector.unwrap();
        assert_eq!(sel.head.parts, vec![SimpleSelector::Class("active".into())]);
        assert!(sel.tail.is_empty());
    }

    #[test]
    fn parse_simple_id() {
        let q = parse_query("#main").unwrap();
        let sel = q.selector.unwrap();
        assert_eq!(sel.head.parts, vec![SimpleSelector::Id("main".into())]);
    }

    #[test]
    fn parse_simple_tag() {
        let q = parse_query("div").unwrap();
        let sel = q.selector.unwrap();
        assert_eq!(sel.head.parts, vec![SimpleSelector::Tag("div".into())]);
    }

    #[test]
    fn parse_compound_tag_class() {
        let q = parse_query("div.active").unwrap();
        let sel = q.selector.unwrap();
        assert_eq!(
            sel.head.parts,
            vec![SimpleSelector::Tag("div".into()), SimpleSelector::Class("active".into()),]
        );
    }

    #[test]
    fn parse_compound_tag_id_class() {
        let q = parse_query("button#submit.primary").unwrap();
        let sel = q.selector.unwrap();
        assert_eq!(
            sel.head.parts,
            vec![
                SimpleSelector::Tag("button".into()),
                SimpleSelector::Id("submit".into()),
                SimpleSelector::Class("primary".into()),
            ]
        );
    }

    #[test]
    fn parse_descendant_combinator() {
        let q = parse_query(".sidebar .menu-item").unwrap();
        let sel = q.selector.unwrap();
        assert_eq!(sel.head.parts, vec![SimpleSelector::Class("sidebar".into())]);
        assert_eq!(sel.tail.len(), 1);
        assert_eq!(sel.tail[0].0, Combinator::Descendant);
        assert_eq!(sel.tail[0].1.parts, vec![SimpleSelector::Class("menu-item".into())]);
    }

    #[test]
    fn parse_child_combinator() {
        let q = parse_query(".nav > .link").unwrap();
        let sel = q.selector.unwrap();
        assert_eq!(sel.head.parts, vec![SimpleSelector::Class("nav".into())]);
        assert_eq!(sel.tail.len(), 1);
        assert_eq!(sel.tail[0].0, Combinator::Child);
        assert_eq!(sel.tail[0].1.parts, vec![SimpleSelector::Class("link".into())]);
    }

    #[test]
    fn parse_attribute_selector() {
        let q = parse_query("[placeholder=\"Search\"]").unwrap();
        let sel = q.selector.unwrap();
        assert_eq!(
            sel.head.parts,
            vec![SimpleSelector::Attribute { name: "placeholder".into(), value: "Search".into() }]
        );
    }

    #[test]
    fn parse_pseudo_nth_child() {
        let q = parse_query(":nth-child(2)").unwrap();
        let sel = q.selector.unwrap();
        assert_eq!(sel.head.parts, vec![SimpleSelector::PseudoClass(PseudoClass::NthChild(2))]);
    }

    #[test]
    fn parse_pseudo_first_child() {
        let q = parse_query(":first-child").unwrap();
        let sel = q.selector.unwrap();
        assert_eq!(sel.head.parts, vec![SimpleSelector::PseudoClass(PseudoClass::FirstChild)]);
    }

    #[test]
    fn parse_text_exact() {
        let q = parse_query("text(\"Click me\")").unwrap();
        assert!(q.selector.is_none());
        assert_eq!(q.text, Some(TextMatcher::Exact("Click me".into())));
    }

    #[test]
    fn parse_text_contains() {
        let q = parse_query("has_text(\"Click\")").unwrap();
        assert!(q.selector.is_none());
        assert_eq!(q.text, Some(TextMatcher::Contains("Click".into())));
    }

    #[test]
    fn parse_empty_fails() {
        assert!(parse_query("").is_err());
    }

    #[test]
    fn parse_leading_combinator_fails() {
        assert!(parse_query("> .foo").is_err());
    }

    #[test]
    fn parse_multi_chain() {
        let q = parse_query(".a > .b .c").unwrap();
        let sel = q.selector.unwrap();
        assert_eq!(sel.tail.len(), 2);
        assert_eq!(sel.tail[0].0, Combinator::Child);
        assert_eq!(sel.tail[1].0, Combinator::Descendant);
    }

    #[test]
    fn parse_pseudo_checked() {
        let q = parse_query(":checked").unwrap();
        let sel = q.selector.unwrap();
        assert_eq!(sel.head.parts, vec![SimpleSelector::PseudoClass(PseudoClass::Checked)]);
    }

    #[test]
    fn parse_input_type_attr() {
        let q = parse_query("input[type=\"checkbox\"]").unwrap();
        let sel = q.selector.unwrap();
        assert_eq!(
            sel.head.parts,
            vec![
                SimpleSelector::Tag("input".into()),
                SimpleSelector::Attribute { name: "type".into(), value: "checkbox".into() },
            ]
        );
    }
}
