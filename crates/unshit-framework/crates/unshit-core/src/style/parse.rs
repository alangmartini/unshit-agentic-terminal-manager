use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::cursor::CursorShape;
use crate::style::transition::{TimingFunction, TransitionDef, TransitionProperty};
use crate::style::types;
use crate::style::types::*;
use cssparser::{Parser, ParserInput, Token};
use smallvec::SmallVec;

#[derive(Debug, Clone, Default)]
pub struct CompiledStylesheet {
    pub rules: Vec<CompiledRule>,
    pub custom_properties: HashMap<String, String>,
    /// `@font-face` rules collected in source order. Consumed by the app
    /// crate's font loader at startup to register fonts with cosmic-text.
    pub font_faces: Vec<FontFaceRule>,
    /// `@keyframes` rules, keyed by animation name (case sensitive, matching
    /// the CSS spec). One table is shared across the whole stylesheet so the
    /// animation driver can resolve names at tick time.
    pub keyframes: HashMap<String, KeyframesRule>,
}

#[derive(Debug, Clone)]
pub struct CompiledRule {
    pub selector: SelectorChain,
    pub specificity: (u16, u16, u16),
    pub declarations: Vec<StyleDeclaration>,
    pub source_order: u32,
}

/// Parsed `@font-face { font-family: ...; src: ...; }` rule.
///
/// This type carries no IO. File loading happens in the app crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFaceRule {
    /// The family name this face registers as, from `font-family:`.
    pub family: String,
    /// The resolved source descriptor from the first `src:` entry.
    pub src: FontFaceSrc,
}

/// Source side of a parsed `@font-face` rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontFaceSrc {
    /// `src: url("path/to/font.ttf")`. Relative paths resolve against the
    /// app working directory at load time. `data:` URIs are rejected during
    /// loading, not parsing.
    Url(String),
    /// `src: local("Family Name")`. Recorded for completeness and for the
    /// future fallback chain wiring. Ignored at load time.
    Local(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectorPart {
    Universal,
    Tag(String),
    Class(String),
    Id(String),
    PseudoClass(PseudoClass),
    PseudoElement(PseudoElement),
}

#[derive(Debug, Clone)]
pub enum SelectorCombinator {
    Descendant,
    Child,
}

#[derive(Debug, Clone)]
pub struct SelectorChain {
    pub parts: Vec<(Vec<SelectorPart>, Option<SelectorCombinator>)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PseudoClass {
    Hover,
    Active,
    Focus,
    FocusVisible,
    FocusWithin,
    FirstChild,
    LastChild,
    NthChild(i32),
    Not(Box<SelectorPart>),
}

/// Pseudo elements that can be synthesized as arena children of a host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PseudoElement {
    Before,
    After,
    Selection,
    Placeholder,
}

impl SelectorChain {
    /// Returns the `PseudoElement` that this selector targets, if any.
    /// Only the last compound selector's tail is inspected.
    pub fn pseudo_element(&self) -> Option<PseudoElement> {
        let (last_parts, _) = self.parts.last()?;
        last_parts.iter().find_map(|part| match part {
            SelectorPart::PseudoElement(pe) => Some(*pe),
            _ => None,
        })
    }
}

/// A single `box-shadow` layer as seen at parse time.
///
/// `color` is optional: when the CSS value omits the color, the parser stores
/// `None` and the resolver fills it in from the element's `color` at apply
/// time. This matches the CSS behavior where the default is `currentColor`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParsedBoxShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub spread_radius: f32,
    pub color: Option<Color>,
    pub inset: bool,
}

#[derive(Debug, Clone)]
pub enum StyleDeclaration {
    Content(ContentValue),
    Display(Display),
    FlexDirection(FlexDirection),
    FlexGrow(f32),
    FlexShrink(f32),
    FlexBasis(Dimension),
    AlignItems(AlignItems),
    AlignSelf(AlignSelf),
    JustifyContent(JustifyContent),
    FlexWrap(FlexWrap),
    AlignContent(AlignContent),
    Width(Dimension),
    Height(Dimension),
    MinWidth(Dimension),
    MinHeight(Dimension),
    MaxWidth(Dimension),
    MaxHeight(Dimension),
    Padding(Edges),
    PaddingTop(f32),
    PaddingRight(f32),
    PaddingBottom(f32),
    PaddingLeft(f32),
    Margin(Edges),
    MarginTop(f32),
    MarginRight(f32),
    MarginBottom(f32),
    MarginLeft(f32),
    Gap(f32),
    RowGap(f32),
    ColumnGap(f32),
    Overflow(Overflow),
    Background(types::Background),
    BorderColor(Color),
    BorderWidth(Edges),
    BorderRadius(Corners),
    Opacity(f32),
    BoxShadowList(SmallVec<[ParsedBoxShadow; 2]>),
    BackdropFilter(types::BackdropFilter),
    Color(Color),
    FontSize(f32),
    FontWeight(FontWeight),
    FontFamily(String),
    LineHeight(f32),
    LetterSpacing(f32),
    TextAlign(TextAlign),
    TextDecoration(TextDecoration),
    TextDecorationColor(Color),
    WhiteSpace(types::WhiteSpace),
    Cursor(CursorStyle),
    Visibility(Visibility),
    PointerEvents(PointerEvents),
    Position(CssPosition),
    Top(Dimension),
    Right(Dimension),
    Bottom(Dimension),
    Left(Dimension),
    OutlineColor(Color),
    OutlineWidth(f32),
    OutlineOffset(f32),
    Layer(types::Layer),
    RenderTarget(types::Layer),
    CaretColor(Color),
    CaretShape(CursorShape),
    CaretBlinkRate(u32),
    PlaceholderColor(Color),
    Transition(SmallVec<[TransitionDef; 2]>),

    // Animations (shorthand and longhands). The shorthand and the
    // `Animation` variant overwrite any previously applied animation set;
    // the longhands act on the cascaded animation list field by field, CSS
    // style, by producing one declaration per entry.
    Animation(SmallVec<[types::AnimationDef; 2]>),
    AnimationName(SmallVec<[Option<Arc<str>>; 2]>),
    AnimationDuration(SmallVec<[Duration; 2]>),
    AnimationTimingFunction(SmallVec<[TimingFunction; 2]>),
    AnimationDelay(SmallVec<[(Duration, i64); 2]>),
    AnimationIterationCount(SmallVec<[types::IterationCount; 2]>),
    AnimationDirection(SmallVec<[types::AnimationDirection; 2]>),
    AnimationFillMode(SmallVec<[types::AnimationFillMode; 2]>),
    AnimationPlayState(SmallVec<[types::AnimationPlayState; 2]>),

    // Keyboard capture
    KeyboardCapture(bool),

    // Grid container properties
    GridTemplateColumns(Vec<types::GridTrackDef>),
    GridTemplateRows(Vec<types::GridTrackDef>),
    GridAutoColumns(Vec<types::GridTrackSize>),
    GridAutoRows(Vec<types::GridTrackSize>),
    GridAutoFlow(types::GridAutoFlow),

    // Grid item properties
    GridColumnStart(types::GridPlacement),
    GridColumnEnd(types::GridPlacement),
    GridRowStart(types::GridPlacement),
    GridRowEnd(types::GridPlacement),

    // User select
    UserSelect(UserSelect),

    // Resize handle
    ResizeAxis(crate::resize_handle::ResizeAxis),

    // Bell / notification
    BellStyle(types::BellStyle),
}

impl CompiledStylesheet {
    pub fn parse(css: &str) -> Self {
        let custom_properties = extract_custom_properties(css);
        let resolved_css = resolve_var_references(css, &custom_properties);

        let mut input = ParserInput::new(&resolved_css);
        let mut parser = Parser::new(&mut input);
        let mut rules = Vec::new();
        let mut font_faces = Vec::new();
        let mut keyframes: HashMap<String, KeyframesRule> = HashMap::new();
        let mut source_order = 0u32;

        while !parser.is_exhausted() {
            // Peek: is this an at-rule? If so, dispatch. Otherwise fall
            // through to the selector rule path. The peek is done through
            // a saved parser state so the tokens remain available to the
            // rule parser on the non-at-rule branch.
            let state_before = parser.state();
            let is_at_rule = matches!(parser.next(), Ok(Token::AtKeyword(_)));
            parser.reset(&state_before);

            if is_at_rule {
                // Consume the at-keyword and branch on its name.
                let name = match parser.next() {
                    Ok(Token::AtKeyword(name)) => name.clone(),
                    _ => {
                        // Unreachable under normal parsing, but stay safe.
                        continue;
                    }
                };

                if name.eq_ignore_ascii_case("font-face") {
                    match parse_font_face(&mut parser) {
                        Ok(rule) => font_faces.push(rule),
                        Err(()) => {
                            // Best effort: skip the malformed block and keep
                            // parsing the rest of the stylesheet.
                            skip_at_rule_body(&mut parser);
                        }
                    }
                } else if name.eq_ignore_ascii_case("keyframes") {
                    match parse_keyframes(&mut parser) {
                        Ok(rule) => {
                            // Later definitions of the same name overwrite
                            // earlier ones, matching browser behavior.
                            keyframes.insert(rule.name.clone(), rule);
                        }
                        Err(()) => skip_at_rule_body(&mut parser),
                    }
                } else {
                    // Forward compat: unknown at-rules are skipped without
                    // breaking the rest of the stylesheet.
                    skip_at_rule_body(&mut parser);
                }
                continue;
            }

            // parse_rule always drains its block on failure, so no
            // extra token skip is needed on the error path.
            if let Ok(rule) = parse_rule(&mut parser, source_order) {
                rules.push(rule);
                source_order += 1;
            }
        }

        rules.sort_by(|a, b| {
            a.specificity.cmp(&b.specificity).then(a.source_order.cmp(&b.source_order))
        });

        CompiledStylesheet { rules, custom_properties, font_faces, keyframes }
    }
}

/// Consume and discard the contents of the current nested block. Call this
/// right after the parser has seen a block-opener token (curly brace,
/// parenthesis, square bracket, or function) to keep the parser consistent.
fn drain_nested_block(parser: &mut Parser) {
    let _ = parser.parse_nested_block(|p| -> Result<(), cssparser::ParseError<'_, ()>> {
        drain_tokens(p);
        Ok(())
    });
}

/// Drain all remaining tokens in the parser, recursively descending into any
/// nested blocks or functions. This respects cssparser's invariant that
/// block-opener tokens must immediately be followed by `parse_nested_block`.
fn drain_tokens(parser: &mut Parser) {
    while !parser.is_exhausted() {
        match parser.next() {
            Ok(Token::Function(_))
            | Ok(Token::ParenthesisBlock)
            | Ok(Token::SquareBracketBlock)
            | Ok(Token::CurlyBracketBlock) => drain_nested_block(parser),
            Ok(_) => continue,
            Err(_) => return,
        }
    }
}

/// Skip to the end of an at-rule: consume tokens until the next curly
/// bracket block (which we then step past) or the next top level semicolon.
fn skip_at_rule_body(parser: &mut Parser) {
    while !parser.is_exhausted() {
        match parser.next() {
            Ok(Token::Semicolon) => return,
            Ok(Token::CurlyBracketBlock) => {
                drain_nested_block(parser);
                return;
            }
            Ok(Token::Function(_))
            | Ok(Token::ParenthesisBlock)
            | Ok(Token::SquareBracketBlock) => drain_nested_block(parser),
            Ok(_) => continue,
            Err(_) => return,
        }
    }
}

/// Parse the body of a `@font-face` at-rule. The `@font-face` keyword itself
/// has already been consumed by the caller. This function consumes the
/// block (`{ ... }`) and returns a populated [`FontFaceRule`].
///
/// The contract with cssparser is strict: once we see a `CurlyBracketBlock`
/// token we must call `parse_nested_block` on the very next operation, or
/// the tokenizer panics. That is why we drive the loop as a state machine
/// here instead of peeking with `reset`.
fn parse_font_face(parser: &mut Parser) -> Result<FontFaceRule, ()> {
    // Walk forward until we hit the opening curly bracket block. @font-face
    // has no prelude in CSS3, but we skip any spurious tokens defensively.
    loop {
        match parser.next() {
            Ok(Token::CurlyBracketBlock) => break,
            Ok(Token::Semicolon) => return Err(()),
            Ok(_) => continue,
            Err(_) => return Err(()),
        }
    }

    let mut family: Option<String> = None;
    let mut src: Option<FontFaceSrc> = None;

    let parse_result: Result<(), cssparser::ParseError<'_, ()>> = parser.parse_nested_block(|p| {
        while !p.is_exhausted() {
            // Read the descriptor name.
            let descriptor = match p.next() {
                Ok(Token::Ident(name)) => name.to_string(),
                Ok(Token::Semicolon) => continue,
                Ok(_) => {
                    // Skip to the next semicolon or end of block.
                    skip_to_semicolon(p);
                    continue;
                }
                Err(_) => break,
            };

            if p.expect_colon().is_err() {
                skip_to_semicolon(p);
                continue;
            }

            match descriptor.to_ascii_lowercase().as_str() {
                "font-family" => {
                    match p.next() {
                        Ok(Token::QuotedString(s)) => family = Some(s.to_string()),
                        Ok(Token::Ident(s)) => family = Some(s.to_string()),
                        _ => {}
                    }
                    skip_to_semicolon(p);
                }
                "src" => {
                    // Read the first src entry. A full CSS3 `src` list can
                    // include multiple fallbacks separated by commas, but
                    // phase 1 only honors the first workable entry.
                    if let Ok(parsed) = parse_font_face_src(p) {
                        if src.is_none() {
                            src = Some(parsed);
                        }
                    }
                    skip_to_semicolon(p);
                }
                _ => {
                    // Ignore unknown descriptors (e.g. font-weight,
                    // font-style, unicode-range, font-display).
                    skip_to_semicolon(p);
                }
            }
        }
        Ok(())
    });

    parse_result.map_err(|_| ())?;

    match (family, src) {
        (Some(family), Some(src)) if !family.is_empty() => Ok(FontFaceRule { family, src }),
        _ => Err(()),
    }
}

/// Parse a single `src:` value: `url("path")` or `local("Name")`. Consumes
/// tokens up to but not including any trailing comma or semicolon.
fn parse_font_face_src(parser: &mut Parser) -> Result<FontFaceSrc, ()> {
    match parser.next().map_err(|_| ())? {
        Token::Function(name) => {
            let name = name.clone();
            let body = parser
                .parse_nested_block(|p| -> Result<FontFaceSrc, cssparser::ParseError<'_, ()>> {
                    let ident_or_string = match p.next() {
                        Ok(Token::QuotedString(s)) => s.to_string(),
                        Ok(Token::Ident(s)) => s.to_string(),
                        _ => return Err(p.new_custom_error(())),
                    };
                    if name.eq_ignore_ascii_case("url") {
                        Ok(FontFaceSrc::Url(ident_or_string))
                    } else if name.eq_ignore_ascii_case("local") {
                        Ok(FontFaceSrc::Local(ident_or_string))
                    } else {
                        Err(p.new_custom_error(()))
                    }
                })
                .map_err(|_: cssparser::ParseError<'_, ()>| ())?;
            Ok(body)
        }
        Token::UnquotedUrl(s) => Ok(FontFaceSrc::Url(s.to_string())),
        _ => Err(()),
    }
}

/// Advance the parser to the next top level semicolon or the end of the
/// current block, consuming the semicolon if found. Nested blocks or
/// functions are descended into and drained so that cssparser's
/// invariant on block-opener tokens is not violated.
fn skip_to_semicolon(parser: &mut Parser) {
    while !parser.is_exhausted() {
        match parser.next() {
            Ok(Token::Semicolon) => return,
            Ok(Token::Function(_))
            | Ok(Token::ParenthesisBlock)
            | Ok(Token::SquareBracketBlock)
            | Ok(Token::CurlyBracketBlock) => drain_nested_block(parser),
            Ok(_) => continue,
            Err(_) => return,
        }
    }
}

/// Extract custom property declarations (--name: value) from :root and * rules.
/// Scans the raw CSS text to find rule blocks with :root or * selectors,
/// then parses `--name: value;` declarations within them.
fn extract_custom_properties(css: &str) -> HashMap<String, String> {
    let mut props = HashMap::new();
    let mut search_start = 0;

    while search_start < css.len() {
        let brace_open = match css[search_start..].find('{') {
            Some(pos) => search_start + pos,
            None => break,
        };

        let selector = css[search_start..brace_open].trim();

        let brace_close = match find_matching_brace(&css[brace_open..]) {
            Some(pos) => brace_open + pos,
            None => break,
        };

        let is_root = selector == ":root" || selector == "*";

        if is_root {
            let block = &css[brace_open + 1..brace_close];
            for decl in block.split(';') {
                let decl = decl.trim();
                if decl.is_empty() {
                    continue;
                }
                if let Some(colon_pos) = decl.find(':') {
                    let name = decl[..colon_pos].trim();
                    let value = decl[colon_pos + 1..].trim();
                    if name.starts_with("--") {
                        props.insert(name.to_string(), value.to_string());
                    }
                }
            }
        }

        search_start = brace_close + 1;
    }

    props
}

/// Find the position of the matching closing brace, starting from a '{'.
fn find_matching_brace(s: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, ch) in s.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Replace all `var(--name)` and `var(--name, fallback)` occurrences in the CSS text.
/// Iterates to handle variables whose values themselves contain var() references.
fn resolve_var_references(css: &str, props: &HashMap<String, String>) -> String {
    if !css.contains("var(") {
        return css.to_string();
    }

    let mut result = css.to_string();
    for _ in 0..10 {
        match resolve_var_once(&result, props) {
            Some(new_css) => result = new_css,
            None => break,
        }
    }

    result
}

/// Single pass of var() substitution. Returns None if no substitutions were made.
fn resolve_var_once(css: &str, props: &HashMap<String, String>) -> Option<String> {
    let prefix = "var(";
    let _ = css.find(prefix)?;

    let mut result = String::with_capacity(css.len());
    let mut remaining = css;
    let mut changed = false;

    while let Some(pos) = remaining.find(prefix) {
        result.push_str(&remaining[..pos]);
        let after_var = &remaining[pos + prefix.len()..];

        if let Some((var_content, rest)) = extract_balanced_parens(after_var) {
            let var_content = var_content.trim();

            let (var_name, fallback) = if let Some(comma_pos) = find_top_level_comma(var_content) {
                let name = var_content[..comma_pos].trim();
                let fb = var_content[comma_pos + 1..].trim();
                (name, Some(fb))
            } else {
                (var_content, None)
            };

            if let Some(value) = props.get(var_name) {
                result.push_str(value);
                changed = true;
            } else if let Some(fb) = fallback {
                result.push_str(fb);
                changed = true;
            } else {
                // No resolution possible, keep the var() call as-is
                result.push_str(prefix);
                result.push_str(var_content);
                result.push(')');
            }

            remaining = rest;
        } else {
            // Malformed var(), keep as-is
            result.push_str(prefix);
            remaining = after_var;
        }
    }

    result.push_str(remaining);

    if changed {
        Some(result)
    } else {
        None
    }
}

/// Extract content inside balanced parentheses. Returns (content, rest_after_close_paren).
fn extract_balanced_parens(s: &str) -> Option<(&str, &str)> {
    let mut depth = 1;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((&s[..i], &s[i + 1..]));
                }
            }
            _ => {}
        }
    }
    None
}

/// Find the position of the first comma that is not inside nested parentheses.
fn find_top_level_comma(s: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

fn parse_rule(parser: &mut Parser, source_order: u32) -> Result<CompiledRule, ()> {
    let selector_str = collect_selector_text(parser)?;
    let selector = match parse_selector_string(&selector_str) {
        Ok(s) => s,
        Err(()) => {
            // collect_selector_text already consumed the CurlyBracketBlock
            // token; drain its contents to keep the parser consistent.
            drain_nested_block(parser);
            return Err(());
        }
    };
    let specificity = compute_specificity(&selector);

    let declarations = parser
        .parse_nested_block(|parser| {
            let mut decls = Vec::new();
            while !parser.is_exhausted() {
                if let Ok(parsed) = parse_declaration(parser) {
                    decls.extend(parsed);
                } else {
                    while let Ok(token) = parser.next() {
                        if matches!(token, Token::Semicolon) {
                            break;
                        }
                    }
                }
            }
            Ok(decls)
        })
        .map_err(|_: cssparser::ParseError<'_, ()>| ())?;

    Ok(CompiledRule { selector, specificity, declarations, source_order })
}

fn collect_selector_text(parser: &mut Parser) -> Result<String, ()> {
    let start = parser.position();
    loop {
        match parser.next() {
            Ok(Token::CurlyBracketBlock) => {
                let slice = parser.slice_from(start);
                let selector_text = slice.trim().trim_end_matches('{').trim();
                return Ok(selector_text.to_string());
            }
            Ok(_) => continue,
            Err(_) => return Err(()),
        }
    }
}

fn parse_selector_string(s: &str) -> Result<SelectorChain, ()> {
    let mut parts = Vec::new();
    let mut current_parts = Vec::new();

    let tokens: Vec<&str> = s.split_whitespace().collect();
    for (i, token) in tokens.iter().enumerate() {
        if *token == ">" {
            if !current_parts.is_empty() {
                parts.push((current_parts, Some(SelectorCombinator::Child)));
                current_parts = Vec::new();
            }
            continue;
        }

        let mut sub_parts = parse_simple_selector(token)?;
        current_parts.append(&mut sub_parts);

        if i < tokens.len() - 1 && tokens[i + 1] != ">" {
            parts.push((current_parts, Some(SelectorCombinator::Descendant)));
            current_parts = Vec::new();
        }
    }

    if !current_parts.is_empty() {
        parts.push((current_parts, None));
    }

    Ok(SelectorChain { parts })
}

fn consume_parenthesized(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    chars.next(); // consume '('
    let mut arg = String::new();
    while let Some(&c) = chars.peek() {
        if c == ')' {
            chars.next();
            break;
        }
        arg.push(c);
        chars.next();
    }
    arg
}

fn parse_simple_selector(s: &str) -> Result<Vec<SelectorPart>, ()> {
    let mut parts = Vec::new();
    let mut chars = s.chars().peekable();
    let mut buf = String::new();

    while let Some(&ch) = chars.peek() {
        match ch {
            '.' => {
                if !buf.is_empty() {
                    parts.push(SelectorPart::Tag(buf.clone()));
                    buf.clear();
                }
                chars.next();
                while let Some(&c) = chars.peek() {
                    if c == '.' || c == '#' || c == ':' {
                        break;
                    }
                    buf.push(c);
                    chars.next();
                }
                if !buf.is_empty() {
                    parts.push(SelectorPart::Class(buf.clone()));
                    buf.clear();
                }
            }
            '#' => {
                if !buf.is_empty() {
                    parts.push(SelectorPart::Tag(buf.clone()));
                    buf.clear();
                }
                chars.next();
                while let Some(&c) = chars.peek() {
                    if c == '.' || c == '#' || c == ':' {
                        break;
                    }
                    buf.push(c);
                    chars.next();
                }
                if !buf.is_empty() {
                    parts.push(SelectorPart::Id(buf.clone()));
                    buf.clear();
                }
            }
            ':' => {
                if !buf.is_empty() {
                    parts.push(SelectorPart::Tag(buf.clone()));
                    buf.clear();
                }
                chars.next();
                // Detect `::` for pseudo elements. A single `:` followed by a
                // `before` or `after` identifier is also honored for legacy
                // CSS1 compatibility.
                let double_colon = chars.peek() == Some(&':');
                if double_colon {
                    chars.next();
                }
                while let Some(&c) = chars.peek() {
                    if c == '.' || c == '#' || c == ':' || c == '(' {
                        break;
                    }
                    buf.push(c);
                    chars.next();
                }
                let has_parens = chars.peek() == Some(&'(');
                match buf.as_str() {
                    "before" => parts.push(SelectorPart::PseudoElement(PseudoElement::Before)),
                    "after" => parts.push(SelectorPart::PseudoElement(PseudoElement::After)),
                    "selection" | "-moz-selection" => {
                        parts.push(SelectorPart::PseudoElement(PseudoElement::Selection));
                    }
                    "placeholder" | "-webkit-input-placeholder" | "-moz-placeholder"
                        if double_colon =>
                    {
                        parts.push(SelectorPart::PseudoElement(PseudoElement::Placeholder));
                    }
                    _ if double_colon => {
                        // Unknown pseudo element (e.g. vendor-prefixed like
                        // ::-webkit-scrollbar, ::-moz-*): reject the entire
                        // selector so the rule is discarded. Previously this
                        // silently dropped the pseudo part, which caused the
                        // remaining selector to match the host element and
                        // misapply declarations.
                        return Err(());
                    }
                    "hover" => parts.push(SelectorPart::PseudoClass(PseudoClass::Hover)),
                    "active" => parts.push(SelectorPart::PseudoClass(PseudoClass::Active)),
                    "focus" => parts.push(SelectorPart::PseudoClass(PseudoClass::Focus)),
                    "focus-visible" => {
                        parts.push(SelectorPart::PseudoClass(PseudoClass::FocusVisible))
                    }
                    "focus-within" => {
                        parts.push(SelectorPart::PseudoClass(PseudoClass::FocusWithin))
                    }
                    "first-child" => parts.push(SelectorPart::PseudoClass(PseudoClass::FirstChild)),
                    "last-child" => parts.push(SelectorPart::PseudoClass(PseudoClass::LastChild)),
                    // :root matches the root element; treat as universal for matching.
                    "root" => parts.push(SelectorPart::Universal),
                    "nth-child" if has_parens => {
                        let arg = consume_parenthesized(&mut chars);
                        if let Ok(n) = arg.trim().parse::<i32>() {
                            parts.push(SelectorPart::PseudoClass(PseudoClass::NthChild(n)));
                        }
                    }
                    "not" if has_parens => {
                        let arg = consume_parenthesized(&mut chars);
                        let inner = arg.trim();
                        if let Some(class) = inner.strip_prefix('.') {
                            parts.push(SelectorPart::PseudoClass(PseudoClass::Not(Box::new(
                                SelectorPart::Class(class.to_string()),
                            ))));
                        } else if let Some(id) = inner.strip_prefix('#') {
                            parts.push(SelectorPart::PseudoClass(PseudoClass::Not(Box::new(
                                SelectorPart::Id(id.to_string()),
                            ))));
                        } else if !inner.is_empty() {
                            parts.push(SelectorPart::PseudoClass(PseudoClass::Not(Box::new(
                                SelectorPart::Tag(inner.to_string()),
                            ))));
                        }
                    }
                    _ => {}
                }
                buf.clear();
            }
            '*' => {
                chars.next();
                parts.push(SelectorPart::Universal);
            }
            _ => {
                buf.push(ch);
                chars.next();
            }
        }
    }

    if !buf.is_empty() {
        parts.push(SelectorPart::Tag(buf));
    }

    if parts.is_empty() {
        return Err(());
    }
    Ok(parts)
}

fn compute_specificity(chain: &SelectorChain) -> (u16, u16, u16) {
    let mut ids = 0u16;
    let mut classes = 0u16;
    let mut tags = 0u16;

    for (parts, _) in &chain.parts {
        for part in parts {
            match part {
                SelectorPart::Id(_) => ids += 1,
                SelectorPart::Class(_) | SelectorPart::PseudoClass(_) => classes += 1,
                // Pseudo elements count as one tag level unit, matching the
                // behavior of modern browsers.
                SelectorPart::Tag(_) | SelectorPart::PseudoElement(_) => tags += 1,
                SelectorPart::Universal => {}
            }
        }
    }

    (ids, classes, tags)
}

fn parse_declaration(parser: &mut Parser) -> Result<SmallVec<[StyleDeclaration; 2]>, ()> {
    let property = parser.expect_ident().map_err(|_| ())?.to_string();
    parser.expect_colon().map_err(|_| ())?;

    let decl = match property.as_str() {
        "display" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::Display(match val.as_ref() {
                "flex" => Display::Flex,
                "block" => Display::Block,
                "inline-flex" => Display::InlineFlex,
                "inline-block" => Display::InlineBlock,
                "grid" => Display::Grid,
                "none" => Display::None,
                _ => return Err(()),
            })
        }
        "flex-direction" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::FlexDirection(match val.as_ref() {
                "row" => FlexDirection::Row,
                "column" => FlexDirection::Column,
                "row-reverse" => FlexDirection::RowReverse,
                "column-reverse" => FlexDirection::ColumnReverse,
                _ => return Err(()),
            })
        }
        "flex-grow" => StyleDeclaration::FlexGrow(parse_number(parser)?),
        "flex-shrink" => StyleDeclaration::FlexShrink(parse_number(parser)?),
        "flex-basis" => StyleDeclaration::FlexBasis(parse_dimension(parser)?),
        "flex" => {
            // flex: none
            if let Ok(ident) = parser
                .try_parse(|p| p.expect_ident().map(|s| s.as_ref().to_string()).map_err(|_| ()))
            {
                if ident == "none" {
                    let _ = parser.try_parse(cssparser::Parser::expect_semicolon);
                    return Ok(smallvec::smallvec![
                        StyleDeclaration::FlexGrow(0.0),
                        StyleDeclaration::FlexShrink(0.0),
                        StyleDeclaration::FlexBasis(Dimension::Auto),
                    ]);
                }
                return Err(());
            }

            let grow = parse_number(parser)?;
            let shrink = parser.try_parse(|p| parse_number(p));

            match shrink {
                Ok(shrink_val) => {
                    // flex: <number> <number> [<dimension>]
                    let basis =
                        parser.try_parse(|p| parse_dimension(p)).unwrap_or(Dimension::Px(0.0));
                    let _ = parser.try_parse(cssparser::Parser::expect_semicolon);
                    return Ok(smallvec::smallvec![
                        StyleDeclaration::FlexGrow(grow),
                        StyleDeclaration::FlexShrink(shrink_val),
                        StyleDeclaration::FlexBasis(basis),
                    ]);
                }
                Err(_) => {
                    // flex: <number>
                    let _ = parser.try_parse(cssparser::Parser::expect_semicolon);
                    return Ok(smallvec::smallvec![
                        StyleDeclaration::FlexGrow(grow),
                        StyleDeclaration::FlexShrink(1.0),
                        StyleDeclaration::FlexBasis(Dimension::Px(0.0)),
                    ]);
                }
            }
        }
        "align-items" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::AlignItems(match val.as_ref() {
                "flex-start" | "start" => AlignItems::Start,
                "flex-end" | "end" => AlignItems::End,
                "center" => AlignItems::Center,
                "stretch" => AlignItems::Stretch,
                "baseline" => AlignItems::Baseline,
                _ => return Err(()),
            })
        }
        "align-self" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::AlignSelf(match val.as_ref() {
                "auto" => AlignSelf::Auto,
                "flex-start" | "start" => AlignSelf::Start,
                "flex-end" | "end" => AlignSelf::End,
                "center" => AlignSelf::Center,
                "stretch" => AlignSelf::Stretch,
                "baseline" => AlignSelf::Baseline,
                _ => return Err(()),
            })
        }
        "justify-content" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::JustifyContent(match val.as_ref() {
                "flex-start" | "start" => JustifyContent::Start,
                "flex-end" | "end" => JustifyContent::End,
                "center" => JustifyContent::Center,
                "space-between" => JustifyContent::SpaceBetween,
                "space-around" => JustifyContent::SpaceAround,
                "space-evenly" => JustifyContent::SpaceEvenly,
                _ => return Err(()),
            })
        }
        "flex-wrap" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::FlexWrap(match val.as_ref() {
                "nowrap" => FlexWrap::NoWrap,
                "wrap" => FlexWrap::Wrap,
                "wrap-reverse" => FlexWrap::WrapReverse,
                _ => return Err(()),
            })
        }
        "align-content" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::AlignContent(match val.as_ref() {
                "flex-start" | "start" => AlignContent::Start,
                "flex-end" | "end" => AlignContent::End,
                "center" => AlignContent::Center,
                "stretch" => AlignContent::Stretch,
                "space-between" => AlignContent::SpaceBetween,
                "space-around" => AlignContent::SpaceAround,
                "space-evenly" => AlignContent::SpaceEvenly,
                _ => return Err(()),
            })
        }
        "width" => StyleDeclaration::Width(parse_dimension(parser)?),
        "height" => StyleDeclaration::Height(parse_dimension(parser)?),
        "min-width" => StyleDeclaration::MinWidth(parse_dimension(parser)?),
        "min-height" => StyleDeclaration::MinHeight(parse_dimension(parser)?),
        "max-width" => StyleDeclaration::MaxWidth(parse_dimension(parser)?),
        "max-height" => StyleDeclaration::MaxHeight(parse_dimension(parser)?),
        "padding" => StyleDeclaration::Padding(parse_edges(parser)?),
        "padding-top" => StyleDeclaration::PaddingTop(parse_px(parser)?),
        "padding-right" => StyleDeclaration::PaddingRight(parse_px(parser)?),
        "padding-bottom" => StyleDeclaration::PaddingBottom(parse_px(parser)?),
        "padding-left" => StyleDeclaration::PaddingLeft(parse_px(parser)?),
        "margin" => StyleDeclaration::Margin(parse_edges(parser)?),
        "margin-top" => StyleDeclaration::MarginTop(parse_px(parser)?),
        "margin-right" => StyleDeclaration::MarginRight(parse_px(parser)?),
        "margin-bottom" => StyleDeclaration::MarginBottom(parse_px(parser)?),
        "margin-left" => StyleDeclaration::MarginLeft(parse_px(parser)?),
        "gap" => {
            let first = parse_px(parser)?;
            let second = parser.try_parse(|p| parse_px(p)).unwrap_or(first);
            let _ = parser.try_parse(cssparser::Parser::expect_semicolon);
            return Ok(smallvec::smallvec![
                StyleDeclaration::RowGap(first),
                StyleDeclaration::ColumnGap(second),
            ]);
        }
        "row-gap" => StyleDeclaration::RowGap(parse_px(parser)?),
        "column-gap" => StyleDeclaration::ColumnGap(parse_px(parser)?),
        "overflow" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::Overflow(match val.as_ref() {
                "visible" => Overflow::Visible,
                "hidden" => Overflow::Hidden,
                "scroll" => Overflow::Scroll,
                _ => return Err(()),
            })
        }
        "background" => match parser.try_parse(|p| parse_linear_gradient(p)) {
            Ok(gradient) => {
                StyleDeclaration::Background(types::Background::LinearGradient(gradient))
            }
            Err(_) => match parser.try_parse(|p| parse_radial_gradient(p)) {
                Ok(gradient) => {
                    StyleDeclaration::Background(types::Background::RadialGradient(gradient))
                }
                Err(_) => {
                    StyleDeclaration::Background(types::Background::Color(parse_color(parser)?))
                }
            },
        },
        // `background-image` only accepts image sources (gradients, url()),
        // never solid colors. We currently honor the gradient path and
        // defer other image sources to later work.
        "background-image" => {
            let gradient = parse_linear_gradient(parser)?;
            StyleDeclaration::Background(types::Background::LinearGradient(gradient))
        }
        "background-color" => {
            StyleDeclaration::Background(types::Background::Color(parse_color(parser)?))
        }
        "border-color" => StyleDeclaration::BorderColor(parse_color(parser)?),
        "border-width" => StyleDeclaration::BorderWidth(parse_edges(parser)?),
        "border-radius" => StyleDeclaration::BorderRadius(parse_corners(parser)?),
        "opacity" => StyleDeclaration::Opacity(parse_number(parser)?),
        "color" => StyleDeclaration::Color(parse_color(parser)?),
        "font-size" => StyleDeclaration::FontSize(parse_px(parser)?),
        "font-weight" => {
            let tok = parser.next().map_err(|_| ())?;
            let w = match tok {
                Token::Ident(ref s) => match s.as_ref() {
                    "normal" => FontWeight::Normal,
                    "bold" => FontWeight::Bold,
                    _ => return Err(()),
                },
                Token::Number { int_value: Some(n), .. } => FontWeight::W(*n as u16),
                _ => return Err(()),
            };
            StyleDeclaration::FontWeight(w)
        }
        "font-family" => {
            let val = parser.expect_ident_or_string().map_err(|_| ())?;
            StyleDeclaration::FontFamily(val.as_ref().to_string())
        }
        "content" => StyleDeclaration::Content(parse_content_value(parser)?),
        "line-height" => StyleDeclaration::LineHeight(parse_number(parser)?),
        "text-align" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::TextAlign(match val.as_ref() {
                "left" => TextAlign::Left,
                "center" => TextAlign::Center,
                "right" => TextAlign::Right,
                _ => return Err(()),
            })
        }
        "text-decoration" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::TextDecoration(match val.as_ref() {
                "none" => TextDecoration::None,
                "underline" => TextDecoration::Underline,
                "line-through" => TextDecoration::LineThrough,
                "overline" => TextDecoration::Overline,
                _ => return Err(()),
            })
        }
        "text-decoration-color" => StyleDeclaration::TextDecorationColor(parse_color(parser)?),
        "white-space" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::WhiteSpace(match val.as_ref() {
                "normal" => types::WhiteSpace::Normal,
                "nowrap" => types::WhiteSpace::Nowrap,
                "pre" => types::WhiteSpace::Pre,
                "pre-wrap" => types::WhiteSpace::PreWrap,
                "pre-line" => types::WhiteSpace::PreLine,
                _ => return Err(()),
            })
        }
        "cursor" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::Cursor(match val.as_ref() {
                "default" | "auto" => CursorStyle::Default,
                "none" => CursorStyle::None,
                "pointer" => CursorStyle::Pointer,
                "text" => CursorStyle::Text,
                "grab" => CursorStyle::Grab,
                "grabbing" => CursorStyle::Grabbing,
                "not-allowed" => CursorStyle::NotAllowed,
                "crosshair" => CursorStyle::Crosshair,
                "move" => CursorStyle::Move,
                "wait" => CursorStyle::Wait,
                "help" => CursorStyle::Help,
                "progress" => CursorStyle::Progress,
                "col-resize" => CursorStyle::ColResize,
                "row-resize" => CursorStyle::RowResize,
                "n-resize" => CursorStyle::NResize,
                "s-resize" => CursorStyle::SResize,
                "e-resize" => CursorStyle::EResize,
                "w-resize" => CursorStyle::WResize,
                "ne-resize" => CursorStyle::NeResize,
                "nw-resize" => CursorStyle::NwResize,
                "se-resize" => CursorStyle::SeResize,
                "sw-resize" => CursorStyle::SwResize,
                "ew-resize" => CursorStyle::EwResize,
                "ns-resize" => CursorStyle::NsResize,
                "nesw-resize" => CursorStyle::NeswResize,
                "nwse-resize" => CursorStyle::NwseResize,
                "zoom-in" => CursorStyle::ZoomIn,
                "zoom-out" => CursorStyle::ZoomOut,
                _ => return Err(()),
            })
        }
        "visibility" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::Visibility(match val.as_ref() {
                "visible" => Visibility::Visible,
                "hidden" => Visibility::Hidden,
                _ => return Err(()),
            })
        }
        "pointer-events" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::PointerEvents(match val.as_ref() {
                "auto" => PointerEvents::Auto,
                "none" => PointerEvents::None,
                _ => return Err(()),
            })
        }
        "user-select" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::UserSelect(match val.as_ref() {
                "auto" => UserSelect::Auto,
                "none" => UserSelect::None,
                "text" => UserSelect::Text,
                "all" => UserSelect::All,
                _ => return Err(()),
            })
        }
        "keyboard-capture" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::KeyboardCapture(match val.as_ref() {
                "none" => false,
                "all" => true,
                _ => return Err(()),
            })
        }
        "position" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::Position(match val.as_ref() {
                "static" => CssPosition::Static,
                "relative" => CssPosition::Relative,
                "absolute" => CssPosition::Absolute,
                _ => return Err(()),
            })
        }
        "top" => StyleDeclaration::Top(parse_dimension(parser)?),
        "right" => StyleDeclaration::Right(parse_dimension(parser)?),
        "bottom" => StyleDeclaration::Bottom(parse_dimension(parser)?),
        "left" => StyleDeclaration::Left(parse_dimension(parser)?),
        "inset" => {
            // CSS `inset` shorthand expands to top, right, bottom, left.
            // Only the single-value form (`inset: <value>`) is supported;
            // multi-value forms are not yet handled.
            let dim = parse_dimension(parser)?;
            let _ = parser.try_parse(cssparser::Parser::expect_semicolon);
            return Ok(smallvec::smallvec![
                StyleDeclaration::Top(dim),
                StyleDeclaration::Right(dim),
                StyleDeclaration::Bottom(dim),
                StyleDeclaration::Left(dim),
            ]);
        }
        "letter-spacing" => StyleDeclaration::LetterSpacing(parse_px(parser)?),
        "box-shadow" => StyleDeclaration::BoxShadowList(parse_box_shadow_list(parser)?),
        "backdrop-filter" => {
            // backdrop-filter: none | blur(<length>) [, <filter-function>]*
            // Only `blur()` entries are honored today. Other filter functions
            // parse to no op entries so a stylesheet listing them still
            // produces a valid declaration.
            match parse_backdrop_filter(parser)? {
                Some(bf) => StyleDeclaration::BackdropFilter(bf),
                None => {
                    let _ = parser.try_parse(cssparser::Parser::expect_semicolon);
                    return Ok(SmallVec::new());
                }
            }
        }
        "outline" => {
            let width = parse_px(parser)?;
            let color = parse_color(parser)?;
            let _ = parser.try_parse(|p| p.expect_semicolon());
            return Ok(smallvec::smallvec![
                StyleDeclaration::OutlineWidth(width),
                StyleDeclaration::OutlineColor(color),
            ]);
        }
        "outline-color" => StyleDeclaration::OutlineColor(parse_color(parser)?),
        "outline-width" => StyleDeclaration::OutlineWidth(parse_px(parser)?),
        "outline-offset" => StyleDeclaration::OutlineOffset(parse_px(parser)?),
        "layer" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::Layer(parse_layer_name(val.as_ref())?)
        }
        "render-target" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::RenderTarget(parse_layer_name(val.as_ref())?)
        }
        "caret-color" => StyleDeclaration::CaretColor(parse_color(parser)?),
        "caret-shape" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            let shape = match val.as_ref() {
                "block" => CursorShape::Block,
                "beam" => CursorShape::Beam,
                "underline" => CursorShape::Underline,
                _ => return Err(()),
            };
            StyleDeclaration::CaretShape(shape)
        }
        "caret-blink-rate" => {
            // Parse integer milliseconds (e.g., `530`)
            let val = parse_px(parser)?;
            StyleDeclaration::CaretBlinkRate(val.max(0.0) as u32)
        }
        "placeholder-color" => StyleDeclaration::PlaceholderColor(parse_color(parser)?),
        "transition" => {
            let defs = parse_transition_shorthand(parser)?;
            StyleDeclaration::Transition(defs)
        }

        // Animation shorthand and longhands. See parse_animation_shorthand
        // for parse rules.
        "animation" => {
            let defs = parse_animation_shorthand(parser)?;
            StyleDeclaration::Animation(defs)
        }
        "animation-name" => {
            let mut list: SmallVec<[Option<Arc<str>>; 2]> = SmallVec::new();
            loop {
                let ident = parser.expect_ident().map_err(|_| ())?;
                let name = if ident.eq_ignore_ascii_case("none") {
                    None
                } else {
                    Some(Arc::<str>::from(ident.as_ref()))
                };
                list.push(name);
                if parser.try_parse(cssparser::Parser::expect_comma).is_err() {
                    break;
                }
            }
            StyleDeclaration::AnimationName(list)
        }
        "animation-duration" => {
            let mut list: SmallVec<[Duration; 2]> = SmallVec::new();
            loop {
                let dur = parse_time_value(parser)?;
                list.push(dur);
                if parser.try_parse(cssparser::Parser::expect_comma).is_err() {
                    break;
                }
            }
            StyleDeclaration::AnimationDuration(list)
        }
        "animation-timing-function" => {
            let mut list: SmallVec<[TimingFunction; 2]> = SmallVec::new();
            loop {
                let tf = parse_timing_function(parser)?;
                list.push(tf);
                if parser.try_parse(cssparser::Parser::expect_comma).is_err() {
                    break;
                }
            }
            StyleDeclaration::AnimationTimingFunction(list)
        }
        "animation-delay" => {
            let mut list: SmallVec<[(Duration, i64); 2]> = SmallVec::new();
            loop {
                let entry = parse_signed_time_value(parser)?;
                list.push(entry);
                if parser.try_parse(cssparser::Parser::expect_comma).is_err() {
                    break;
                }
            }
            StyleDeclaration::AnimationDelay(list)
        }
        "animation-iteration-count" => {
            let mut list: SmallVec<[types::IterationCount; 2]> = SmallVec::new();
            loop {
                let ic = parse_iteration_count(parser)?;
                list.push(ic);
                if parser.try_parse(cssparser::Parser::expect_comma).is_err() {
                    break;
                }
            }
            StyleDeclaration::AnimationIterationCount(list)
        }
        "animation-direction" => {
            let mut list: SmallVec<[types::AnimationDirection; 2]> = SmallVec::new();
            loop {
                let ident = parser.expect_ident().map_err(|_| ())?;
                let d = animation_direction_from_ident(&ident.to_ascii_lowercase()).ok_or(())?;
                list.push(d);
                if parser.try_parse(cssparser::Parser::expect_comma).is_err() {
                    break;
                }
            }
            StyleDeclaration::AnimationDirection(list)
        }
        "animation-fill-mode" => {
            let mut list: SmallVec<[types::AnimationFillMode; 2]> = SmallVec::new();
            loop {
                let ident = parser.expect_ident().map_err(|_| ())?;
                let f = animation_fill_from_ident(&ident.to_ascii_lowercase()).ok_or(())?;
                list.push(f);
                if parser.try_parse(cssparser::Parser::expect_comma).is_err() {
                    break;
                }
            }
            StyleDeclaration::AnimationFillMode(list)
        }
        "animation-play-state" => {
            let mut list: SmallVec<[types::AnimationPlayState; 2]> = SmallVec::new();
            loop {
                let ident = parser.expect_ident().map_err(|_| ())?;
                let ps = animation_play_state_from_ident(&ident.to_ascii_lowercase()).ok_or(())?;
                list.push(ps);
                if parser.try_parse(cssparser::Parser::expect_comma).is_err() {
                    break;
                }
            }
            StyleDeclaration::AnimationPlayState(list)
        }

        // Grid container properties
        "grid-template-columns" => {
            StyleDeclaration::GridTemplateColumns(parse_grid_track_list(parser)?)
        }
        "grid-template-rows" => StyleDeclaration::GridTemplateRows(parse_grid_track_list(parser)?),
        "grid-auto-columns" => {
            StyleDeclaration::GridAutoColumns(parse_grid_auto_track_list(parser)?)
        }
        "grid-auto-rows" => StyleDeclaration::GridAutoRows(parse_grid_auto_track_list(parser)?),
        "grid-auto-flow" => {
            let first = parser.expect_ident().map_err(|_| ())?.to_string();
            let second = parser.try_parse(|p| p.expect_ident().map(|s| s.to_string()));
            let flow = match (first.as_str(), second.as_ref().map(|s| s.as_str())) {
                ("row", Ok("dense")) | ("dense", Ok("row")) => types::GridAutoFlow::RowDense,
                ("column", Ok("dense")) | ("dense", Ok("column")) => {
                    types::GridAutoFlow::ColumnDense
                }
                ("row", _) => types::GridAutoFlow::Row,
                ("column", _) => types::GridAutoFlow::Column,
                ("dense", _) => types::GridAutoFlow::RowDense,
                _ => return Err(()),
            };
            StyleDeclaration::GridAutoFlow(flow)
        }

        // Grid item properties
        "grid-column-start" => StyleDeclaration::GridColumnStart(parse_grid_placement(parser)?),
        "grid-column-end" => StyleDeclaration::GridColumnEnd(parse_grid_placement(parser)?),
        "grid-row-start" => StyleDeclaration::GridRowStart(parse_grid_placement(parser)?),
        "grid-row-end" => StyleDeclaration::GridRowEnd(parse_grid_placement(parser)?),
        "grid-column" => {
            let start = parse_grid_placement(parser)?;
            let end = if parser.try_parse(|p| p.expect_delim('/')).is_ok() {
                parse_grid_placement(parser)?
            } else {
                types::GridPlacement::Auto
            };
            let _ = parser.try_parse(cssparser::Parser::expect_semicolon);
            return Ok(smallvec::smallvec![
                StyleDeclaration::GridColumnStart(start),
                StyleDeclaration::GridColumnEnd(end),
            ]);
        }
        "grid-row" => {
            let start = parse_grid_placement(parser)?;
            let end = if parser.try_parse(|p| p.expect_delim('/')).is_ok() {
                parse_grid_placement(parser)?
            } else {
                types::GridPlacement::Auto
            };
            let _ = parser.try_parse(cssparser::Parser::expect_semicolon);
            return Ok(smallvec::smallvec![
                StyleDeclaration::GridRowStart(start),
                StyleDeclaration::GridRowEnd(end),
            ]);
        }
        "grid-area" => {
            // grid-area: row-start / column-start / row-end / column-end
            let row_start = parse_grid_placement(parser)?;
            let col_start = if parser.try_parse(|p| p.expect_delim('/')).is_ok() {
                parse_grid_placement(parser)?
            } else {
                types::GridPlacement::Auto
            };
            let row_end = if parser.try_parse(|p| p.expect_delim('/')).is_ok() {
                parse_grid_placement(parser)?
            } else {
                types::GridPlacement::Auto
            };
            let col_end = if parser.try_parse(|p| p.expect_delim('/')).is_ok() {
                parse_grid_placement(parser)?
            } else {
                types::GridPlacement::Auto
            };
            let _ = parser.try_parse(cssparser::Parser::expect_semicolon);
            return Ok(smallvec::smallvec![
                StyleDeclaration::GridRowStart(row_start),
                StyleDeclaration::GridColumnStart(col_start),
                StyleDeclaration::GridRowEnd(row_end),
                StyleDeclaration::GridColumnEnd(col_end),
            ]);
        }

        // Resize handle
        "resize-axis" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::ResizeAxis(match val.as_ref() {
                "vertical" => crate::resize_handle::ResizeAxis::Vertical,
                "horizontal" => crate::resize_handle::ResizeAxis::Horizontal,
                _ => return Err(()),
            })
        }

        // Bell / notification
        "bell-style" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::BellStyle(match val.as_ref() {
                "visual" => types::BellStyle::Visual,
                "attention" => types::BellStyle::Attention,
                "both" => types::BellStyle::Both,
                "none" => types::BellStyle::None,
                _ => return Err(()),
            })
        }

        _ => return Err(()),
    };

    let _ = parser.try_parse(cssparser::Parser::expect_semicolon);

    Ok(smallvec::smallvec![decl])
}

fn parse_layer_name(name: &str) -> Result<types::Layer, ()> {
    match name {
        "background" => Ok(types::Layer::Background),
        "content" => Ok(types::Layer::Content),
        "popover" => Ok(types::Layer::Popover),
        "modal" => Ok(types::Layer::Modal),
        "overlay" => Ok(types::Layer::Overlay),
        "tooltip" => Ok(types::Layer::Tooltip),
        "debug" => Ok(types::Layer::Debug),
        _ => Err(()),
    }
}

fn parse_number(parser: &mut Parser) -> Result<f32, ()> {
    match parser.next().map_err(|_| ())? {
        Token::Number { value, .. } => Ok(*value),
        _ => Err(()),
    }
}

/// Parse a value for the CSS `content` property.
///
/// Accepted forms:
/// - `none` and `normal` keyword idents.
/// - A single quoted string literal.
/// - A function call `attr(ident)` where the argument is a bare identifier.
fn parse_content_value(parser: &mut Parser) -> Result<ContentValue, ()> {
    // Clone the next token so we can own its string data without holding a
    // borrow on the parser state during branching.
    let tok = parser.next().map_err(|_| ())?.clone();
    match tok {
        Token::Ident(ref ident) => match ident.as_ref() {
            "none" => Ok(ContentValue::None),
            "normal" => Ok(ContentValue::Normal),
            _ => Err(()),
        },
        Token::QuotedString(ref s) => Ok(ContentValue::Literal(s.as_ref().to_string())),
        Token::Function(ref name) if name.as_ref().eq_ignore_ascii_case("attr") => parser
            .parse_nested_block(|p| {
                let attr_name =
                    p.expect_ident().map(|s| s.as_ref().to_string()).map_err(|e| e.into());
                attr_name.map(ContentValue::Attr)
            })
            .map_err(|_: cssparser::ParseError<'_, ()>| ()),
        _ => Err(()),
    }
}

fn parse_px(parser: &mut Parser) -> Result<f32, ()> {
    match parser.next().map_err(|_| ())? {
        Token::Dimension { value, .. } => Ok(*value),
        Token::Number { value, .. } => Ok(*value),
        _ => Err(()),
    }
}

/// Maximum allowed blur radius in CSS pixels. Values above this clamp so
/// hostile or accidental stylesheets cannot request extremely wide kernels.
pub(crate) const BACKDROP_FILTER_MAX_BLUR_RADIUS: f32 = 64.0;

/// Parse the value part of a `backdrop-filter` declaration.
///
/// Accepts:
/// * `none` ident, which returns `Ok(None)` so the caller drops the
///   declaration entirely.
/// * A comma separated list of filter functions where only `blur(<length>)`
///   entries are honored. Other filter functions parse as no ops so the rule
///   keeps going. A list that contains zero recognized entries returns
///   `Ok(None)`.
fn parse_backdrop_filter(parser: &mut Parser) -> Result<Option<types::BackdropFilter>, ()> {
    // The `none` keyword takes priority and is mutually exclusive with any
    // filter function entries.
    let is_none = parser
        .try_parse(|p| {
            let ident = p.expect_ident().map_err(|_| ())?;
            if ident.as_ref().eq_ignore_ascii_case("none") {
                Ok(())
            } else {
                Err(())
            }
        })
        .is_ok();
    if is_none {
        return Ok(None);
    }

    let mut filters: SmallVec<[types::FilterFunction; 2]> = SmallVec::new();
    loop {
        // Every entry must be a function token. Recognized names produce an
        // entry in the list, unrecognized ones silently drop so the rest of
        // the rule keeps going.
        let entry = parser.try_parse(|p| -> Result<Option<types::FilterFunction>, ()> {
            let name = match p.next().map_err(|_| ())? {
                Token::Function(name) => name.as_ref().to_ascii_lowercase(),
                _ => return Err(()),
            };
            let result = p.parse_nested_block(
                |inner| -> Result<Option<types::FilterFunction>, cssparser::ParseError<'_, ()>> {
                    if name == "blur" {
                        // Empty `blur()` is a zero radius per CSS Filter Effects.
                        let radius = inner.try_parse(parse_px).unwrap_or(0.0);
                        let clamped = radius.clamp(0.0, BACKDROP_FILTER_MAX_BLUR_RADIUS);
                        Ok(Some(types::FilterFunction::Blur(clamped)))
                    } else {
                        // Non blur filter functions parse to a no op so the rest
                        // of the declaration keeps processing. We still walk the
                        // inner contents so the parser advances past them.
                        while inner.next().is_ok() {}
                        Ok(None)
                    }
                },
            );
            result.map_err(|_| ())
        });
        match entry {
            Ok(Some(f)) => filters.push(f),
            Ok(None) => {}
            Err(_) => break,
        }
        if parser.try_parse(cssparser::Parser::expect_comma).is_err() {
            break;
        }
    }

    if filters.is_empty() {
        Ok(None)
    } else {
        Ok(Some(types::BackdropFilter { filters }))
    }
}

fn parse_dimension(parser: &mut Parser) -> Result<Dimension, ()> {
    match parser.next().map_err(|_| ())? {
        Token::Ident(ref s) if s.as_ref() == "auto" => Ok(Dimension::Auto),
        Token::Dimension { value, unit, .. } => {
            if unit.as_ref() == "%" {
                Ok(Dimension::Percent(*value))
            } else {
                Ok(Dimension::Px(*value))
            }
        }
        Token::Percentage { unit_value, .. } => Ok(Dimension::Percent(*unit_value * 100.0)),
        Token::Number { value, .. } => Ok(Dimension::Px(*value)),
        _ => Err(()),
    }
}

fn parse_px_list(parser: &mut Parser) -> Vec<f32> {
    let mut values = Vec::with_capacity(4);
    while values.len() < 4 {
        match parser.try_parse(|p| parse_px(p)) {
            Ok(v) => values.push(v),
            Err(_) => break,
        }
    }
    values
}

fn parse_edges(parser: &mut Parser) -> Result<Edges, ()> {
    let values = parse_px_list(parser);
    match values.len() {
        1 => Ok(Edges::all(values[0])),
        2 => Ok(Edges { top: values[0], right: values[1], bottom: values[0], left: values[1] }),
        3 => Ok(Edges { top: values[0], right: values[1], bottom: values[2], left: values[1] }),
        4 => Ok(Edges { top: values[0], right: values[1], bottom: values[2], left: values[3] }),
        _ => Err(()),
    }
}

fn parse_corners(parser: &mut Parser) -> Result<Corners, ()> {
    let values = parse_px_list(parser);
    match values.len() {
        1 => Ok(Corners::all(values[0])),
        2 => Ok(Corners {
            top_left: values[0],
            top_right: values[1],
            bottom_right: values[0],
            bottom_left: values[1],
        }),
        4 => Ok(Corners {
            top_left: values[0],
            top_right: values[1],
            bottom_right: values[2],
            bottom_left: values[3],
        }),
        _ => Err(()),
    }
}

/// Parse a single gradient stop position token.
///
/// Accepts three forms:
///   * `<number>%`  percentage, yielding `Percent` in the range [0.0, 1.0].
///   * `<number>px` absolute pixel offset, yielding `Px(value)`.
///   * `<number>` a bare unitless number, treated as pixels so the
///     terminal-manager form `red 0` matches expectations.
///
/// Returns `Err` when the next token is anything else. The caller uses
/// `try_parse` so a failed read leaves the parser cursor intact.
fn try_parse_stop_position(p: &mut Parser) -> Result<types::GradientStopPosition, ()> {
    p.try_parse(|p| match p.next() {
        Ok(Token::Percentage { unit_value, .. }) => {
            Ok(types::GradientStopPosition::Percent(*unit_value))
        }
        Ok(Token::Dimension { value, unit, .. }) if unit.as_ref().eq_ignore_ascii_case("px") => {
            Ok(types::GradientStopPosition::Px(*value))
        }
        Ok(Token::Number { value, .. }) => Ok(types::GradientStopPosition::Px(*value)),
        _ => Err(()),
    })
}

/// Parse a CSS color stop list of the form `<color> <percentage>?` entries
/// separated by commas, then apply the CSS Images Level 3 position fixup.
///
/// This is the shared entry point for both `linear-gradient` and
/// `radial-gradient`. The caller must have already consumed any prefix
/// (angle for linear, shape/size/position for radial) and the separating
/// comma before invoking this helper.
///
/// Returns an error if fewer than two stops are present, which matches the
/// CSS grammar requirement that a gradient has at least two color stops.
fn parse_color_stop_list<'i>(
    p: &mut Parser<'i, '_>,
) -> Result<SmallVec<[types::GradientStop; 4]>, cssparser::ParseError<'i, ()>> {
    // Collect raw (color, optional_position) pairs. None tracks an
    // unspecified position so the fixup pass below can apply the
    // CSS Images Level 3 defaulting rules correctly. Positions can be
    // either percent or pixel values; the fixup and normalization passes
    // below keep each unit separate until batch build time.
    let mut raw: SmallVec<[(types::Color, Option<types::GradientStopPosition>); 4]> =
        SmallVec::new();

    loop {
        let color = parse_color(p).map_err(|_| p.new_custom_error(()))?;
        let pos = p.try_parse(try_parse_stop_position).ok();
        raw.push((color, pos));

        // Continue consuming stops as long as commas separate them.
        if p.try_parse(cssparser::Parser::expect_comma).is_err() {
            break;
        }
    }

    if raw.len() < 2 {
        return Err(p.new_custom_error(()));
    }

    // CSS Images Level 3 position fixup:
    //   1. If the first stop has no position, default to 0 percent.
    //      If the last stop has no position, default to 100 percent.
    //   2. Any intermediate stop with no position is evenly distributed
    //      between the closest preceding and following positioned stops.
    //   3. Clamp each position up to the preceding position so the
    //      final list is monotonically non decreasing.
    let n = raw.len();
    if raw[0].1.is_none() {
        raw[0].1 = Some(types::GradientStopPosition::Percent(0.0));
    }
    if raw[n - 1].1.is_none() {
        raw[n - 1].1 = Some(types::GradientStopPosition::Percent(1.0));
    }

    let mut i = 0;
    while i < n {
        if raw[i].1.is_some() {
            i += 1;
            continue;
        }
        // Find the next positioned stop; evenly distribute the run
        // between `i - 1` (already positioned) and `j`.
        let mut j = i + 1;
        while j < n && raw[j].1.is_none() {
            j += 1;
        }
        let start = raw[i - 1].1.expect("previous stop must be positioned");
        let end = raw[j].1.expect("next stop must be positioned");
        let run_len = (j - i + 1) as f32;
        for k in i..j {
            let step = (k - i + 1) as f32;
            let t = step / run_len;
            let interpolated = interpolate_stop_positions(start, end, t);
            raw[k].1 = Some(interpolated);
        }
        i = j + 1;
    }

    // Monotonic clamp per unit. A pixel stop is clamped against the
    // previous pixel stop's pixel value, a percent stop against the
    // previous percent stop's percent value. Cross unit ordering is
    // enforced later at batch build time once both units have been
    // normalized into [0, 1]. Negative positions clamp to zero per spec.
    let mut prev_percent: f32 = 0.0;
    let mut prev_px: f32 = 0.0;
    for entry in raw.iter_mut() {
        let pos = entry.1.expect("position must be populated after fixup");
        match pos {
            types::GradientStopPosition::Percent(v) => {
                let clamped = v.max(0.0).max(prev_percent);
                entry.1 = Some(types::GradientStopPosition::Percent(clamped));
                prev_percent = clamped;
            }
            types::GradientStopPosition::Px(v) => {
                let clamped = v.max(0.0).max(prev_px);
                entry.1 = Some(types::GradientStopPosition::Px(clamped));
                prev_px = clamped;
            }
        }
    }

    let stops: SmallVec<[types::GradientStop; 4]> = raw
        .into_iter()
        .map(|(color, pos)| types::GradientStop { color, position: pos.unwrap() })
        .collect();

    Ok(stops)
}

fn parse_linear_gradient(parser: &mut Parser) -> Result<types::LinearGradient, ()> {
    // Accept either `linear-gradient` (non repeating) or
    // `repeating-linear-gradient` (tile the stop list along the axis).
    // Both share the exact same stop list grammar; the function name is the
    // only discriminator per CSS Images Level 3.
    let repeating = match parser.next().map_err(|_| ())? {
        Token::Function(ref name) if name.as_ref() == "linear-gradient" => false,
        Token::Function(ref name) if name.as_ref() == "repeating-linear-gradient" => true,
        _ => return Err(()),
    };

    parser
        .parse_nested_block(|p| {
            // Optional leading `<angle>,`. If absent, CSS defaults to 180deg
            // (gradient flows from top to bottom, first stop at the top).
            let angle_deg = p
                .try_parse(|p| match p.next() {
                    Ok(Token::Dimension { value, unit, .. })
                        if unit.as_ref().eq_ignore_ascii_case("deg") =>
                    {
                        let v = *value;
                        match p.expect_comma() {
                            Ok(_) => Ok(v),
                            Err(_) => Err(()),
                        }
                    }
                    _ => Err(()),
                })
                .unwrap_or(180.0);

            let stops = parse_color_stop_list(p)?;

            // Repeating gradients must have a non zero tile span. A zero
            // span would cause a divide by near zero in the shader's
            // `fract` branch.
            if repeating && stops.len() >= 2 {
                let first = stops[0].position;
                let last = stops[stops.len() - 1].position;
                if same_unit_zero_span(first, last) {
                    return Err(p.new_custom_error(()));
                }
            }

            Ok(types::LinearGradient { angle_deg, stops, repeating })
        })
        .map_err(|_: cssparser::ParseError<'_, ()>| ())
}

/// Parse a single `<length-percentage>` token for the radial gradient grammar.
///
/// Accepts `Npx` dimensions, bare numbers (treated as pixels), and
/// percentages. Does not accept `Auto` or unitless values other than zero.
fn parse_length_or_percent<'i>(
    p: &mut Parser<'i, '_>,
) -> Result<types::LengthOrPercent, cssparser::ParseError<'i, ()>> {
    match p.next() {
        Ok(Token::Percentage { unit_value, .. }) => {
            Ok(types::LengthOrPercent::Percent(*unit_value))
        }
        Ok(Token::Dimension { value, unit, .. }) if unit.as_ref().eq_ignore_ascii_case("px") => {
            Ok(types::LengthOrPercent::Px(*value))
        }
        Ok(Token::Number { value, .. }) if *value == 0.0 => Ok(types::LengthOrPercent::Px(0.0)),
        _ => Err(p.new_custom_error(())),
    }
}

/// Parse a single position keyword (`top`, `bottom`, `left`, `right`,
/// `center`) and return the `(x, y)` percentages it implies. Returns an
/// error if the next token is not one of those keywords.
///
/// A returned `None` for an axis means "this keyword did not constrain that
/// axis" (so `top` leaves x free to default to center).
fn parse_position_keyword<'i>(
    p: &mut Parser<'i, '_>,
) -> Result<(Option<f32>, Option<f32>), cssparser::ParseError<'i, ()>> {
    let ident = match p.expect_ident_cloned() {
        Ok(s) => s.as_ref().to_ascii_lowercase(),
        Err(_) => return Err(p.new_custom_error(())),
    };
    match ident.as_str() {
        "left" => Ok((Some(0.0), None)),
        "right" => Ok((Some(1.0), None)),
        "top" => Ok((None, Some(0.0))),
        "bottom" => Ok((None, Some(1.0))),
        "center" => Ok((Some(0.5), Some(0.5))),
        _ => Err(p.new_custom_error(())),
    }
}

/// Parse the `<position>` production after the `at` keyword in a
/// `radial-gradient`. Handles:
///
/// * single keyword: `top`, `bottom`, `left`, `right`, `center`
/// * two keywords in either order: `top left`, `left top`, etc.
/// * two length percent values: `50% 0%`, `10px 20px`
/// * a single length percent value (x only, y defaults to center)
fn parse_radial_position<'i>(
    p: &mut Parser<'i, '_>,
) -> Result<types::RadialPosition, cssparser::ParseError<'i, ()>> {
    // Try the keyword path first: one or two keywords.
    if let Ok(first) = p.try_parse(parse_position_keyword) {
        // Try a second keyword (order independent: `top left` == `left top`).
        let second = p.try_parse(parse_position_keyword).ok();
        let (mut x, mut y) = first;
        if let Some((sx, sy)) = second {
            if let Some(v) = sx {
                x = Some(v);
            }
            if let Some(v) = sy {
                y = Some(v);
            }
        }
        return Ok(types::RadialPosition {
            x: types::LengthOrPercent::Percent(x.unwrap_or(0.5)),
            y: types::LengthOrPercent::Percent(y.unwrap_or(0.5)),
        });
    }

    // Length or percent path. Try to parse one, then optionally a second.
    let x = parse_length_or_percent(p)?;
    let y = p.try_parse(parse_length_or_percent).unwrap_or(types::LengthOrPercent::Percent(0.5));
    Ok(types::RadialPosition { x, y })
}

fn parse_radial_gradient(parser: &mut Parser) -> Result<types::RadialGradient, ()> {
    match parser.next().map_err(|_| ())? {
        Token::Function(ref name) if name.as_ref() == "radial-gradient" => {}
        _ => return Err(()),
    }

    parser
        .parse_nested_block(|p| {
            let mut shape: Option<types::RadialShape> = None;
            let mut size: Option<types::RadialSize> = None;
            let mut center: Option<types::RadialPosition> = None;

            // Optional shape ident. We try `circle` or `ellipse` up front;
            // CSS allows the shape and size to appear in either order, so
            // after each parse we loop back and retry until the head of the
            // stream no longer looks like a shape, a size keyword, or a
            // length.
            loop {
                let made_progress = p
                    .try_parse(|p| -> Result<(), cssparser::ParseError<'_, ()>> {
                        // Shape keyword.
                        if shape.is_none() {
                            if let Ok(ident) = p.try_parse(|p| p.expect_ident_cloned()) {
                                match ident.as_ref().to_ascii_lowercase().as_str() {
                                    "circle" => {
                                        shape = Some(types::RadialShape::Circle);
                                        return Ok(());
                                    }
                                    "ellipse" => {
                                        shape = Some(types::RadialShape::Ellipse);
                                        return Ok(());
                                    }
                                    "closest-side" if size.is_none() => {
                                        size = Some(types::RadialSize::ClosestSide);
                                        return Ok(());
                                    }
                                    "closest-corner" if size.is_none() => {
                                        size = Some(types::RadialSize::ClosestCorner);
                                        return Ok(());
                                    }
                                    "farthest-side" if size.is_none() => {
                                        size = Some(types::RadialSize::FarthestSide);
                                        return Ok(());
                                    }
                                    "farthest-corner" if size.is_none() => {
                                        size = Some(types::RadialSize::FarthestCorner);
                                        return Ok(());
                                    }
                                    _ => return Err(p.new_custom_error(())),
                                }
                            }
                        } else if size.is_none() {
                            // Shape already present; still accept a size keyword.
                            if let Ok(ident) = p.try_parse(|p| p.expect_ident_cloned()) {
                                match ident.as_ref().to_ascii_lowercase().as_str() {
                                    "closest-side" => {
                                        size = Some(types::RadialSize::ClosestSide);
                                        return Ok(());
                                    }
                                    "closest-corner" => {
                                        size = Some(types::RadialSize::ClosestCorner);
                                        return Ok(());
                                    }
                                    "farthest-side" => {
                                        size = Some(types::RadialSize::FarthestSide);
                                        return Ok(());
                                    }
                                    "farthest-corner" => {
                                        size = Some(types::RadialSize::FarthestCorner);
                                        return Ok(());
                                    }
                                    _ => return Err(p.new_custom_error(())),
                                }
                            }
                        }

                        // Explicit size: one length percent (circle) or two
                        // length percents (ellipse).
                        if size.is_none() {
                            if let Ok(first) = p.try_parse(parse_length_or_percent) {
                                // Reject negative explicit sizes per CSS spec.
                                let negative = |lp: types::LengthOrPercent| match lp {
                                    types::LengthOrPercent::Px(v) => v < 0.0,
                                    types::LengthOrPercent::Percent(v) => v < 0.0,
                                };
                                if negative(first) {
                                    return Err(p.new_custom_error(()));
                                }
                                if let Ok(second) = p.try_parse(parse_length_or_percent) {
                                    if negative(second) {
                                        return Err(p.new_custom_error(()));
                                    }
                                    size =
                                        Some(types::RadialSize::Explicit { rx: first, ry: second });
                                } else {
                                    // Single value: only valid for a circle.
                                    // CSS also forbids single length percent
                                    // for an implicit ellipse, but we accept
                                    // it as `rx == ry` and let the shape
                                    // default flag whether it collapses.
                                    size =
                                        Some(types::RadialSize::Explicit { rx: first, ry: first });
                                    // If the shape has not been explicitly
                                    // set, a lone length implies a circle.
                                    if shape.is_none() {
                                        shape = Some(types::RadialShape::Circle);
                                    }
                                }
                                return Ok(());
                            }
                        }

                        Err(p.new_custom_error(()))
                    })
                    .is_ok();

                if !made_progress {
                    break;
                }
            }

            // Optional `at <position>`.
            if p.try_parse(|p| p.expect_ident_matching("at")).is_ok() {
                center = Some(parse_radial_position(p)?);
            }

            // The head of the gradient (shape, size, position) is separated
            // from the stop list by a comma. If no prefix was parsed the
            // entire function body is just stops and no leading comma is
            // present.
            if shape.is_some() || size.is_some() || center.is_some() {
                p.expect_comma().map_err(|_| p.new_custom_error(()))?;
            }

            let stops = parse_color_stop_list(p)?;

            Ok(types::RadialGradient {
                shape: shape.unwrap_or(types::RadialShape::Ellipse),
                size: size.unwrap_or(types::RadialSize::FarthestCorner),
                center: center.unwrap_or(types::RadialPosition::CENTER),
                stops,
            })
        })
        .map_err(|_: cssparser::ParseError<'_, ()>| ())
}

/// Interpolate between two gradient stop positions at parameter `t` in
/// `[0.0, 1.0]`. If both endpoints share a unit the interpolation stays in
/// that unit; when they differ the later endpoint's unit wins, with the
/// starting numeric value projected into that unit trivially (px start
/// maps to pixel distance 0, percent start maps to percent 0) so the
/// placed stop still sits between them.
fn interpolate_stop_positions(
    start: types::GradientStopPosition,
    end: types::GradientStopPosition,
    t: f32,
) -> types::GradientStopPosition {
    use types::GradientStopPosition::{Percent, Px};
    match (start, end) {
        (Percent(a), Percent(b)) => Percent(a + (b - a) * t),
        (Px(a), Px(b)) => Px(a + (b - a) * t),
        (Percent(_), Px(b)) => Px(b * t),
        (Px(_), Percent(b)) => Percent(b * t),
    }
}

/// Return `true` when two positions share a unit and have identical
/// numeric values, used to reject zero length repeating gradient tiles at
/// parse time.
fn same_unit_zero_span(a: types::GradientStopPosition, b: types::GradientStopPosition) -> bool {
    use types::GradientStopPosition::{Percent, Px};
    match (a, b) {
        (Percent(x), Percent(y)) => (y - x).abs() < 1e-6,
        (Px(x), Px(y)) => (y - x).abs() < 1e-6,
        _ => false,
    }
}

fn parse_color(parser: &mut Parser) -> Result<Color, ()> {
    match parser.next().map_err(|_| ())? {
        Token::Hash(ref hash) | Token::IDHash(ref hash) => {
            let s = hash.as_ref();
            parse_hex_color(s)
        }
        Token::Ident(ref name) => match name.as_ref() {
            "transparent" => Ok(Color::TRANSPARENT),
            "white" => Ok(Color::WHITE),
            "black" => Ok(Color::BLACK),
            "red" => Ok(Color::rgb(255, 0, 0)),
            "green" => Ok(Color::rgb(0, 128, 0)),
            "blue" => Ok(Color::rgb(0, 0, 255)),
            "yellow" => Ok(Color::rgb(255, 255, 0)),
            "gray" | "grey" => Ok(Color::rgb(128, 128, 128)),
            "cyan" | "aqua" => Ok(Color::rgb(0, 255, 255)),
            "fuchsia" | "magenta" => Ok(Color::rgb(255, 0, 255)),
            "orange" => Ok(Color::rgb(255, 165, 0)),
            "purple" => Ok(Color::rgb(128, 0, 128)),
            "pink" => Ok(Color::rgb(255, 192, 203)),
            "lime" => Ok(Color::rgb(0, 255, 0)),
            "navy" => Ok(Color::rgb(0, 0, 128)),
            "teal" => Ok(Color::rgb(0, 128, 128)),
            "silver" => Ok(Color::rgb(192, 192, 192)),
            "crimson" => Ok(Color::rgb(220, 20, 60)),
            "coral" => Ok(Color::rgb(255, 127, 80)),
            "gold" => Ok(Color::rgb(255, 215, 0)),
            "indigo" => Ok(Color::rgb(75, 0, 130)),
            "violet" => Ok(Color::rgb(238, 130, 238)),
            "salmon" => Ok(Color::rgb(250, 128, 114)),
            "tomato" => Ok(Color::rgb(255, 99, 71)),
            "turquoise" => Ok(Color::rgb(64, 224, 208)),
            "skyblue" => Ok(Color::rgb(135, 206, 235)),
            _ => Err(()),
        },
        Token::Function(ref name) if name.as_ref() == "rgb" || name.as_ref() == "rgba" => parser
            .parse_nested_block(|p| {
                let r = parse_color_component(p).map_err(|_| p.new_custom_error(()))?;
                let _ = p.try_parse(cssparser::Parser::expect_comma);
                let g = parse_color_component(p).map_err(|_| p.new_custom_error(()))?;
                let _ = p.try_parse(cssparser::Parser::expect_comma);
                let b = parse_color_component(p).map_err(|_| p.new_custom_error(()))?;
                let a = if p.try_parse(cssparser::Parser::expect_comma).is_ok() {
                    parse_alpha_component(p).map_err(|_| p.new_custom_error(()))?
                } else {
                    255
                };
                Ok(Color::rgba(r, g, b, a))
            })
            .map_err(|_: cssparser::ParseError<'_, ()>| ()),
        _ => Err(()),
    }
}

fn parse_color_component(parser: &mut Parser) -> Result<u8, ()> {
    match parser.next().map_err(|_| ())? {
        Token::Number { value, .. } => Ok((*value as i32).clamp(0, 255) as u8),
        Token::Percentage { unit_value, .. } => Ok((*unit_value * 255.0).clamp(0.0, 255.0) as u8),
        _ => Err(()),
    }
}

fn parse_alpha_component(parser: &mut Parser) -> Result<u8, ()> {
    match parser.next().map_err(|_| ())? {
        Token::Number { value, .. } => {
            if *value <= 1.0 {
                Ok((*value * 255.0).clamp(0.0, 255.0) as u8)
            } else {
                Ok((*value as i32).clamp(0, 255) as u8)
            }
        }
        Token::Percentage { unit_value, .. } => Ok((*unit_value * 255.0).clamp(0.0, 255.0) as u8),
        _ => Err(()),
    }
}

fn parse_hex_color(s: &str) -> Result<Color, ()> {
    let hex = |c: u8| -> Result<u8, ()> {
        match c {
            b'0'..=b'9' => Ok(c - b'0'),
            b'a'..=b'f' => Ok(c - b'a' + 10),
            b'A'..=b'F' => Ok(c - b'A' + 10),
            _ => Err(()),
        }
    };

    let bytes = s.as_bytes();
    match bytes.len() {
        3 => {
            let r = hex(bytes[0])?;
            let g = hex(bytes[1])?;
            let b = hex(bytes[2])?;
            Ok(Color::rgb(r << 4 | r, g << 4 | g, b << 4 | b))
        }
        6 => {
            let r = hex(bytes[0])? << 4 | hex(bytes[1])?;
            let g = hex(bytes[2])? << 4 | hex(bytes[3])?;
            let b = hex(bytes[4])? << 4 | hex(bytes[5])?;
            Ok(Color::rgb(r, g, b))
        }
        8 => {
            let r = hex(bytes[0])? << 4 | hex(bytes[1])?;
            let g = hex(bytes[2])? << 4 | hex(bytes[3])?;
            let b = hex(bytes[4])? << 4 | hex(bytes[5])?;
            let a = hex(bytes[6])? << 4 | hex(bytes[7])?;
            Ok(Color::rgba(r, g, b, a))
        }
        _ => Err(()),
    }
}

// ---------------------------------------------------------------------------
// Box-shadow parsing
// ---------------------------------------------------------------------------

/// Parse the `box-shadow` value: either `none` or a comma-separated list of
/// shadow layers.
///
/// Each layer is:
///     [inset] <offset-x> <offset-y> [<blur-radius>] [<spread-radius>] [<color>] [inset]
///
/// The `inset` keyword is accepted either before or after the length values.
/// The color is optional and defaults to `currentColor`, which is resolved at
/// apply time from the element's own `color` field.
fn parse_box_shadow_list(parser: &mut Parser) -> Result<SmallVec<[ParsedBoxShadow; 2]>, ()> {
    let mut layers: SmallVec<[ParsedBoxShadow; 2]> = SmallVec::new();

    // `box-shadow: none` produces an empty list.
    if parser
        .try_parse(|p| {
            let ident = p.expect_ident().map_err(|_| ())?;
            if ident.as_ref().eq_ignore_ascii_case("none") {
                Ok(())
            } else {
                Err(())
            }
        })
        .is_ok()
    {
        return Ok(layers);
    }

    loop {
        let layer = parse_single_box_shadow(parser)?;
        layers.push(layer);

        if parser.try_parse(cssparser::Parser::expect_comma).is_err() {
            break;
        }
    }

    Ok(layers)
}

/// Parse a single box-shadow layer.
fn parse_single_box_shadow(parser: &mut Parser) -> Result<ParsedBoxShadow, ()> {
    let mut inset = false;

    // Optional leading `inset` keyword.
    if parser
        .try_parse(|p| {
            let ident = p.expect_ident().map_err(|_| ())?;
            if ident.as_ref().eq_ignore_ascii_case("inset") {
                Ok(())
            } else {
                Err(())
            }
        })
        .is_ok()
    {
        inset = true;
    }

    // Required offsets.
    let offset_x = parse_px(parser)?;
    let offset_y = parse_px(parser)?;

    // Optional blur and spread. Both default to 0.
    let blur_radius = parser.try_parse(|p| parse_px(p)).unwrap_or(0.0);
    let spread_radius = parser.try_parse(|p| parse_px(p)).unwrap_or(0.0);

    // Optional trailing `inset` keyword. Record it but only overwrite if not
    // already set from the leading position.
    if !inset
        && parser
            .try_parse(|p| {
                let ident = p.expect_ident().map_err(|_| ())?;
                if ident.as_ref().eq_ignore_ascii_case("inset") {
                    Ok(())
                } else {
                    Err(())
                }
            })
            .is_ok()
    {
        inset = true;
    }

    // Optional color. If omitted, the resolver will fall back to the element
    // color at apply time.
    let color = parser.try_parse(|p| parse_color(p)).ok();

    // Trailing `inset` after color is also permitted by some authors.
    if !inset
        && parser
            .try_parse(|p| {
                let ident = p.expect_ident().map_err(|_| ())?;
                if ident.as_ref().eq_ignore_ascii_case("inset") {
                    Ok(())
                } else {
                    Err(())
                }
            })
            .is_ok()
    {
        inset = true;
    }

    Ok(ParsedBoxShadow { offset_x, offset_y, blur_radius, spread_radius, color, inset })
}

// ---------------------------------------------------------------------------
// Transition parsing
// ---------------------------------------------------------------------------

/// Parse the `transition` shorthand: one or more comma-separated entries.
/// Each entry is: `<property> <duration> [<timing-function>] [<delay>]`
///
/// Examples:
///   transition: none;
///   transition: opacity 0.3s ease;
///   transition: all 0.5s cubic-bezier(0.4, 0, 0.2, 1);
///   transition: background 200ms ease-in, opacity 300ms ease-out 50ms;
fn parse_transition_shorthand(parser: &mut Parser) -> Result<SmallVec<[TransitionDef; 2]>, ()> {
    let mut defs = SmallVec::new();

    // Check for "none".
    if parser
        .try_parse(|p| {
            let ident = p.expect_ident().map_err(|_| ())?;
            if ident.as_ref() == "none" {
                Ok(())
            } else {
                Err(())
            }
        })
        .is_ok()
    {
        return Ok(defs); // empty = no transitions
    }

    loop {
        let def = parse_single_transition(parser)?;
        defs.push(def);

        // Try to consume a comma for the next entry.
        if parser.try_parse(cssparser::Parser::expect_comma).is_err() {
            break;
        }
    }

    Ok(defs)
}

/// Parse a single transition entry: `<property> <duration> [<timing-function>] [<delay>]`
fn parse_single_transition(parser: &mut Parser) -> Result<TransitionDef, ()> {
    // Property name.
    let prop_name = parser.expect_ident().map_err(|_| ())?.to_string();
    let property = TransitionProperty::from_str(&prop_name).ok_or(())?;

    // Duration (required).
    let duration = parse_time_value(parser)?;

    // Timing function (optional, defaults to Ease).
    let timing_function = parser.try_parse(parse_timing_function).unwrap_or(TimingFunction::Ease);

    // Delay (optional, defaults to 0).
    let delay = parser.try_parse(parse_time_value).unwrap_or(Duration::ZERO);

    Ok(TransitionDef { property, duration, timing_function, delay })
}

/// Parse a time value: `0.3s`, `300ms`, or `0` (treated as 0s).
fn parse_time_value(parser: &mut Parser) -> Result<Duration, ()> {
    match parser.next().map_err(|_| ())? {
        Token::Dimension { value, unit, .. } => {
            let secs = match unit.as_ref() {
                "s" => *value,
                "ms" => *value / 1000.0,
                _ => return Err(()),
            };
            Ok(Duration::from_secs_f32(secs.max(0.0)))
        }
        Token::Number { value, .. } if *value == 0.0 => Ok(Duration::ZERO),
        _ => Err(()),
    }
}

/// Parse a timing function: `ease`, `linear`, `ease-in`, `ease-out`, `ease-in-out`,
/// or `cubic-bezier(x1, y1, x2, y2)`.
fn parse_timing_function(parser: &mut Parser) -> Result<TimingFunction, ()> {
    // Try a named function first.
    if let Ok(tf) = parser.try_parse(|p| {
        let ident = p.expect_ident().map_err(|_| ())?;
        match ident.as_ref() {
            "linear" => Ok(TimingFunction::Linear),
            "ease" => Ok(TimingFunction::Ease),
            "ease-in" => Ok(TimingFunction::EaseIn),
            "ease-out" => Ok(TimingFunction::EaseOut),
            "ease-in-out" => Ok(TimingFunction::EaseInOut),
            _ => Err(()),
        }
    }) {
        return Ok(tf);
    }

    // Try cubic-bezier(...).
    match parser.next().map_err(|_| ())? {
        Token::Function(ref name) if name.as_ref() == "cubic-bezier" => {}
        _ => return Err(()),
    }

    parser
        .parse_nested_block(|p| {
            let x1 = parse_number(p).map_err(|_| p.new_custom_error(()))?;
            p.expect_comma().map_err(|_| p.new_custom_error(()))?;
            let y1 = parse_number(p).map_err(|_| p.new_custom_error(()))?;
            p.expect_comma().map_err(|_| p.new_custom_error(()))?;
            let x2 = parse_number(p).map_err(|_| p.new_custom_error(()))?;
            p.expect_comma().map_err(|_| p.new_custom_error(()))?;
            let y2 = parse_number(p).map_err(|_| p.new_custom_error(()))?;
            Ok(TimingFunction::CubicBezier(x1, y1, x2, y2))
        })
        .map_err(|_: cssparser::ParseError<'_, ()>| ())
}

// ---------------------------------------------------------------------------
// @keyframes parsing
// ---------------------------------------------------------------------------

/// Parse a full `@keyframes <name> { ... }` block. The `@keyframes` keyword
/// itself has already been consumed by the caller.
///
/// Returns the populated [`KeyframesRule`] with frames sorted by ascending
/// offset. Multi selector blocks like `0%, 100% { opacity: 1; }` are
/// flattened into one entry per offset that share the same declaration list.
fn parse_keyframes(parser: &mut Parser) -> Result<KeyframesRule, ()> {
    // Walk forward looking for the animation name ident, then the opening
    // curly. Whitespace and the CDO/CDC tokens are skipped implicitly by
    // cssparser so we only need to branch on what we see here.
    let mut name: Option<String> = None;
    loop {
        match parser.next() {
            Ok(Token::Ident(ref s)) => {
                // The first ident is the animation name.
                if name.is_none() {
                    name = Some(s.to_string());
                } else {
                    return Err(());
                }
            }
            Ok(Token::QuotedString(ref s)) => {
                if name.is_none() {
                    name = Some(s.to_string());
                } else {
                    return Err(());
                }
            }
            Ok(Token::CurlyBracketBlock) => break,
            Ok(Token::Semicolon) => return Err(()),
            Ok(_) => continue,
            Err(_) => return Err(()),
        }
    }

    let name = name.ok_or(())?;
    let mut frames: Vec<Keyframe> = Vec::new();

    let parse_result: Result<(), cssparser::ParseError<'_, ()>> = parser.parse_nested_block(|p| {
        while !p.is_exhausted() {
            // Try to parse a keyframe selector list (offsets) followed
            // by a declaration block. If either half is malformed we
            // drain to the next block and keep going.
            let selector_state = p.state();
            let offsets = match parse_keyframe_selector(p) {
                Ok(list) if !list.is_empty() => list,
                _ => {
                    p.reset(&selector_state);
                    skip_until_curly_or_end(p);
                    continue;
                }
            };

            // Consume the opening curly. If the next token is not a
            // curly block, skip the entry.
            let opened_block = matches!(p.next(), Ok(Token::CurlyBracketBlock));
            if !opened_block {
                continue;
            }

            let decls_result: Result<Vec<StyleDeclaration>, cssparser::ParseError<'_, ()>> = p
                .parse_nested_block(|block| {
                    let mut decls = Vec::new();
                    while !block.is_exhausted() {
                        if let Ok(parsed) = parse_declaration(block) {
                            decls.extend(parsed);
                        } else {
                            while let Ok(tok) = block.next() {
                                if matches!(tok, Token::Semicolon) {
                                    break;
                                }
                            }
                        }
                    }
                    Ok(decls)
                });

            let decls = match decls_result {
                Ok(d) => d,
                Err(_) => continue,
            };

            for offset in offsets {
                frames.push(Keyframe { offset, declarations: decls.clone() });
            }
        }
        Ok(())
    });

    parse_result.map_err(|_| ())?;

    // Sort by offset so sampling can walk frames monotonically.
    frames.sort_by(|a, b| a.offset.partial_cmp(&b.offset).unwrap_or(std::cmp::Ordering::Equal));

    Ok(KeyframesRule { name, frames })
}

/// Consume tokens until the next curly bracket block closes or the parser is
/// exhausted. Used to recover from a malformed keyframe selector so the rest
/// of the `@keyframes` body can still parse.
fn skip_until_curly_or_end(parser: &mut Parser) {
    while !parser.is_exhausted() {
        match parser.next() {
            Ok(Token::CurlyBracketBlock) => {
                drain_nested_block(parser);
                return;
            }
            Ok(Token::Function(_))
            | Ok(Token::ParenthesisBlock)
            | Ok(Token::SquareBracketBlock) => drain_nested_block(parser),
            Ok(Token::Semicolon) => return,
            Ok(_) => continue,
            Err(_) => return,
        }
    }
}

/// Parse a keyframe selector: `from`, `to`, `<percentage>%`, or a comma
/// separated list of any of those.
///
/// Returns the list of offsets in `0.0..=1.0`. Out of range percentages are
/// clamped. Unknown idents fail the whole selector.
fn parse_keyframe_selector(parser: &mut Parser) -> Result<SmallVec<[f32; 4]>, ()> {
    let mut offsets: SmallVec<[f32; 4]> = SmallVec::new();
    loop {
        match parser.next().map_err(|_| ())? {
            Token::Ident(ref s) => match s.as_ref() {
                // `from` == 0%, `to` == 100%.
                id if id.eq_ignore_ascii_case("from") => offsets.push(0.0),
                id if id.eq_ignore_ascii_case("to") => offsets.push(1.0),
                _ => return Err(()),
            },
            Token::Percentage { unit_value, .. } => {
                let v = unit_value.clamp(0.0, 1.0);
                offsets.push(v);
            }
            Token::Number { value, .. } if *value == 0.0 => {
                offsets.push(0.0);
            }
            _ => return Err(()),
        }

        // A comma continues the list; anything else ends the selector
        // (typically the opening curly brace of the declaration block).
        let commit = parser.state();
        match parser.next() {
            Ok(Token::Comma) => continue,
            Ok(_) => {
                parser.reset(&commit);
                break;
            }
            Err(_) => break,
        }
    }

    if offsets.is_empty() {
        Err(())
    } else {
        Ok(offsets)
    }
}

// ---------------------------------------------------------------------------
// animation shorthand and longhand parsing
// ---------------------------------------------------------------------------

/// Parse the `animation` shorthand: one or more comma separated entries.
///
/// Each entry is an order independent list of the longhand values. Any token
/// that is not recognized as a duration, delay, timing function, iteration
/// count, direction, fill mode, play state, or `none` is treated as the
/// animation name. The first time component is the duration, the second is
/// the delay. This mirrors Blink and Gecko.
///
/// Examples:
/// - `animation: pulse-dot 2s ease-in-out infinite;`
/// - `animation: fade-in 200ms cubic-bezier(0.22, 0.61, 0.36, 1);`
/// - `animation: a 1s, b 2s ease-in 100ms;`
fn parse_animation_shorthand(
    parser: &mut Parser,
) -> Result<SmallVec<[types::AnimationDef; 2]>, ()> {
    let mut defs: SmallVec<[types::AnimationDef; 2]> = SmallVec::new();

    // Special case for `animation: none;` which disables any prior animation.
    if parser
        .try_parse(|p| {
            let ident = p.expect_ident().map_err(|_| ())?;
            if ident.eq_ignore_ascii_case("none") {
                Ok(())
            } else {
                Err(())
            }
        })
        .is_ok()
    {
        defs.push(types::AnimationDef::default());
        return Ok(defs);
    }

    loop {
        let def = parse_single_animation(parser)?;
        defs.push(def);
        if parser.try_parse(cssparser::Parser::expect_comma).is_err() {
            break;
        }
    }

    Ok(defs)
}

/// Parse a single `animation` shorthand entry. Values are order independent,
/// but the first time value is interpreted as the duration and the second as
/// the delay (matching CSS3 Animations).
fn parse_single_animation(parser: &mut Parser) -> Result<types::AnimationDef, ()> {
    let mut def = types::AnimationDef::default();

    let mut duration_set = false;
    let mut delay_set = false;
    let mut timing_set = false;
    let mut iteration_set = false;
    let mut direction_set = false;
    let mut fill_set = false;
    let mut play_state_set = false;
    let mut name_set = false;

    loop {
        // Stop on a comma or end of input, ready for the next entry.
        let probe = parser.state();
        match parser.next() {
            Ok(Token::Comma) => {
                parser.reset(&probe);
                break;
            }
            Ok(Token::Semicolon) => {
                parser.reset(&probe);
                break;
            }
            Err(_) => break,
            Ok(_) => {
                parser.reset(&probe);
            }
        }

        // Try a time value (duration first, then delay).
        if !duration_set || !delay_set {
            if let Ok((dur, signed_ns)) = parser.try_parse(parse_signed_time_value) {
                if !duration_set {
                    def.duration = dur;
                    // Duration cannot be negative; spec clamps it.
                    duration_set = true;
                    continue;
                } else if !delay_set {
                    def.delay = dur;
                    def.delay_nanos = signed_ns;
                    delay_set = true;
                    continue;
                }
            }
        }

        // Timing function.
        if !timing_set {
            if let Ok(tf) = parser.try_parse(parse_timing_function) {
                def.timing_function = tf;
                timing_set = true;
                continue;
            }
        }

        // Iteration count.
        if !iteration_set {
            if let Ok(ic) = parser.try_parse(parse_iteration_count) {
                def.iteration_count = ic;
                iteration_set = true;
                continue;
            }
        }

        // Direction, fill mode, play state, `none`, or name.
        let mut consumed = false;
        let _ = parser.try_parse(|p| -> Result<(), ()> {
            let ident = p.expect_ident().map_err(|_| ())?;
            let lower = ident.to_ascii_lowercase();
            if !direction_set {
                if let Some(d) = animation_direction_from_ident(&lower) {
                    def.direction = d;
                    direction_set = true;
                    consumed = true;
                    return Ok(());
                }
            }
            if !fill_set {
                if let Some(f) = animation_fill_from_ident(&lower) {
                    def.fill_mode = f;
                    fill_set = true;
                    consumed = true;
                    return Ok(());
                }
            }
            if !play_state_set {
                if let Some(p_state) = animation_play_state_from_ident(&lower) {
                    def.play_state = p_state;
                    play_state_set = true;
                    consumed = true;
                    return Ok(());
                }
            }
            if !name_set {
                if lower == "none" {
                    def.name = None;
                } else if lower == "initial" || lower == "inherit" || lower == "unset" {
                    // Reserved CSS globals; treat as no name.
                    def.name = None;
                } else {
                    def.name = Some(Arc::<str>::from(ident.as_ref()));
                }
                name_set = true;
                consumed = true;
                return Ok(());
            }
            Err(())
        });

        if !consumed {
            // Nothing matched; this entry is done.
            break;
        }
    }

    Ok(def)
}

/// Parse a single `animation-iteration-count` value: `infinite` or a number.
fn parse_iteration_count(parser: &mut Parser) -> Result<types::IterationCount, ()> {
    if parser
        .try_parse(|p| {
            let ident = p.expect_ident().map_err(|_| ())?;
            if ident.eq_ignore_ascii_case("infinite") {
                Ok(())
            } else {
                Err(())
            }
        })
        .is_ok()
    {
        return Ok(types::IterationCount::Infinite);
    }

    match parser.next().map_err(|_| ())? {
        Token::Number { value, .. } => Ok(types::IterationCount::Finite((*value).max(0.0))),
        _ => Err(()),
    }
}

fn animation_direction_from_ident(s: &str) -> Option<types::AnimationDirection> {
    match s {
        "normal" => Some(types::AnimationDirection::Normal),
        "reverse" => Some(types::AnimationDirection::Reverse),
        "alternate" => Some(types::AnimationDirection::Alternate),
        "alternate-reverse" => Some(types::AnimationDirection::AlternateReverse),
        _ => None,
    }
}

fn animation_fill_from_ident(s: &str) -> Option<types::AnimationFillMode> {
    match s {
        "none" => Some(types::AnimationFillMode::None),
        "forwards" => Some(types::AnimationFillMode::Forwards),
        "backwards" => Some(types::AnimationFillMode::Backwards),
        "both" => Some(types::AnimationFillMode::Both),
        _ => None,
    }
}

fn animation_play_state_from_ident(s: &str) -> Option<types::AnimationPlayState> {
    match s {
        "running" => Some(types::AnimationPlayState::Running),
        "paused" => Some(types::AnimationPlayState::Paused),
        _ => None,
    }
}

/// Parse a time value that may be negative (required by `animation-delay`).
///
/// Returns a `(Duration, i64_nanos)` pair. The `Duration` is clamped to zero
/// because `std::time::Duration` cannot represent negative values, while the
/// signed nanosecond field preserves the original sign for the driver.
fn parse_signed_time_value(parser: &mut Parser) -> Result<(Duration, i64), ()> {
    match parser.next().map_err(|_| ())? {
        Token::Dimension { value, unit, .. } => {
            let secs = match unit.as_ref() {
                "s" => *value,
                "ms" => *value / 1000.0,
                _ => return Err(()),
            };
            let nanos = (secs * 1_000_000_000.0) as i64;
            let duration = if secs > 0.0 { Duration::from_secs_f32(secs) } else { Duration::ZERO };
            Ok((duration, nanos))
        }
        Token::Number { value, .. } if *value == 0.0 => Ok((Duration::ZERO, 0)),
        _ => Err(()),
    }
}

/// Apply a longhand animation property onto `style.animations`, growing the
/// list if the longhand has more entries than the current list.
fn apply_animation_longhand<F>(style: &mut ComputedStyle, len: usize, mut setter: F)
where
    F: FnMut(&mut types::AnimationDef, usize),
{
    if len == 0 {
        return;
    }
    // Grow the animation list to at least `len` entries so every longhand
    // value has a slot. New slots start from `AnimationDef::default()`.
    while style.animations.len() < len {
        style.animations.push(types::AnimationDef::default());
    }
    for i in 0..style.animations.len() {
        let idx = if len == 0 { 0 } else { i % len };
        setter(&mut style.animations[i], idx);
    }
}

// ---------------------------------------------------------------------------
// Grid track parsing helpers
// ---------------------------------------------------------------------------

/// Parse a single grid track sizing value (e.g., `200px`, `1fr`, `auto`, `min-content`).
fn parse_grid_track_size_single(parser: &mut Parser) -> Result<types::GridTrackSize, ()> {
    // Try ident keywords first
    if let Ok(val) = parser.try_parse(|p| p.expect_ident().map(|s| s.to_string())) {
        return match val.as_str() {
            "auto" => Ok(types::GridTrackSize::auto()),
            "min-content" => Ok(types::GridTrackSize::min_content()),
            "max-content" => Ok(types::GridTrackSize::max_content()),
            _ => Err(()),
        };
    }

    // Try function calls: minmax(), fit-content()
    if let Ok(result) = parser.try_parse(|p| parse_grid_function_track(p)) {
        return Ok(result);
    }

    // Try dimension/number/percentage (includes `fr` unit)
    match parser.next().map_err(|_| ())? {
        Token::Dimension { value, unit, .. } => {
            let unit_str = unit.as_ref();
            if unit_str.eq_ignore_ascii_case("fr") {
                Ok(types::GridTrackSize::fr(*value))
            } else if unit_str == "%" {
                Ok(types::GridTrackSize::fixed_percent(*value))
            } else {
                Ok(types::GridTrackSize::fixed_px(*value))
            }
        }
        Token::Percentage { unit_value, .. } => {
            Ok(types::GridTrackSize::fixed_percent(*unit_value * 100.0))
        }
        Token::Number { value, .. } => Ok(types::GridTrackSize::fixed_px(*value)),
        _ => Err(()),
    }
}

/// Parse a function-form track size: `minmax(...)` or `fit-content(...)`.
fn parse_grid_function_track(parser: &mut Parser) -> Result<types::GridTrackSize, ()> {
    let name = match parser.next().map_err(|_| ())? {
        Token::Function(ref name) => name.to_string(),
        _ => return Err(()),
    };

    match name.as_str() {
        "minmax" => parser
            .parse_nested_block(|p| {
                let min = parse_grid_min_track_size(p).map_err(|_| p.new_custom_error(()))?;
                p.expect_comma().map_err(|_| p.new_custom_error(()))?;
                let max = parse_grid_max_track_size(p).map_err(|_| p.new_custom_error(()))?;
                Ok(types::GridTrackSize::minmax(min, max))
            })
            .map_err(|_: cssparser::ParseError<'_, ()>| ()),
        "fit-content" => parser
            .parse_nested_block(|p| {
                let tok = p.next().cloned().map_err(|_| p.new_custom_error(()))?;
                match tok {
                    Token::Dimension { value, .. } => Ok(types::GridTrackSize {
                        min: types::GridMinTrackSize::Auto,
                        max: types::GridMaxTrackSize::FitContent(value),
                    }),
                    Token::Percentage { unit_value, .. } => Ok(types::GridTrackSize {
                        min: types::GridMinTrackSize::Auto,
                        max: types::GridMaxTrackSize::FitContentPercent(unit_value * 100.0),
                    }),
                    Token::Number { value, .. } => Ok(types::GridTrackSize {
                        min: types::GridMinTrackSize::Auto,
                        max: types::GridMaxTrackSize::FitContent(value),
                    }),
                    _ => Err(p.new_custom_error(())),
                }
            })
            .map_err(|_: cssparser::ParseError<'_, ()>| ()),
        _ => Err(()),
    }
}

/// Parse a min track sizing value (for the first argument of minmax()).
fn parse_grid_min_track_size(parser: &mut Parser) -> Result<types::GridMinTrackSize, ()> {
    if let Ok(val) = parser.try_parse(|p| p.expect_ident().map(|s| s.to_string())) {
        return match val.as_str() {
            "auto" => Ok(types::GridMinTrackSize::Auto),
            "min-content" => Ok(types::GridMinTrackSize::MinContent),
            "max-content" => Ok(types::GridMinTrackSize::MaxContent),
            _ => Err(()),
        };
    }

    match parser.next().map_err(|_| ())? {
        Token::Dimension { value, unit, .. } => {
            if unit.as_ref() == "%" {
                Ok(types::GridMinTrackSize::Percent(*value))
            } else {
                Ok(types::GridMinTrackSize::Px(*value))
            }
        }
        Token::Percentage { unit_value, .. } => {
            Ok(types::GridMinTrackSize::Percent(*unit_value * 100.0))
        }
        Token::Number { value, .. } => Ok(types::GridMinTrackSize::Px(*value)),
        _ => Err(()),
    }
}

/// Parse a max track sizing value (for the second argument of minmax()).
fn parse_grid_max_track_size(parser: &mut Parser) -> Result<types::GridMaxTrackSize, ()> {
    if let Ok(val) = parser.try_parse(|p| p.expect_ident().map(|s| s.to_string())) {
        return match val.as_str() {
            "auto" => Ok(types::GridMaxTrackSize::Auto),
            "min-content" => Ok(types::GridMaxTrackSize::MinContent),
            "max-content" => Ok(types::GridMaxTrackSize::MaxContent),
            _ => Err(()),
        };
    }

    match parser.next().map_err(|_| ())? {
        Token::Dimension { value, unit, .. } => {
            let unit_str = unit.as_ref();
            if unit_str.eq_ignore_ascii_case("fr") {
                Ok(types::GridMaxTrackSize::Fr(*value))
            } else if unit_str == "%" {
                Ok(types::GridMaxTrackSize::Percent(*value))
            } else {
                Ok(types::GridMaxTrackSize::Px(*value))
            }
        }
        Token::Percentage { unit_value, .. } => {
            Ok(types::GridMaxTrackSize::Percent(*unit_value * 100.0))
        }
        Token::Number { value, .. } => Ok(types::GridMaxTrackSize::Px(*value)),
        _ => Err(()),
    }
}

/// Parse a grid template track list (e.g., `200px 1fr minmax(100px, 1fr) repeat(3, 1fr)`).
fn parse_grid_track_list(parser: &mut Parser) -> Result<Vec<types::GridTrackDef>, ()> {
    let mut tracks = Vec::new();

    while !parser.is_exhausted() {
        // Check for semicolon (end of declaration)
        if parser.try_parse(|p| p.expect_semicolon()).is_ok() {
            break;
        }

        // Try repeat() function
        if let Ok(def) = parser.try_parse(|p| parse_grid_repeat(p)) {
            tracks.push(def);
            continue;
        }

        // Try a single track size (including minmax/fit-content functions)
        if let Ok(size) = parser.try_parse(|p| parse_grid_track_size_single(p)) {
            tracks.push(types::GridTrackDef::Single(size));
            continue;
        }

        break;
    }

    if tracks.is_empty() {
        return Err(());
    }

    Ok(tracks)
}

/// Parse a grid auto track list (for grid-auto-columns/grid-auto-rows).
/// These do not support repeat(), only plain track sizes.
fn parse_grid_auto_track_list(parser: &mut Parser) -> Result<Vec<types::GridTrackSize>, ()> {
    let mut tracks = Vec::new();

    while !parser.is_exhausted() {
        if parser.try_parse(|p| p.expect_semicolon()).is_ok() {
            break;
        }

        if let Ok(size) = parser.try_parse(|p| parse_grid_track_size_single(p)) {
            tracks.push(size);
            continue;
        }

        break;
    }

    if tracks.is_empty() {
        return Err(());
    }

    Ok(tracks)
}

/// Parse `repeat(count, track-sizes...)`.
fn parse_grid_repeat(parser: &mut Parser) -> Result<types::GridTrackDef, ()> {
    let name = match parser.next().map_err(|_| ())? {
        Token::Function(ref name) if name.as_ref() == "repeat" => name.to_string(),
        _ => return Err(()),
    };

    if name != "repeat" {
        return Err(());
    }

    parser
        .parse_nested_block(|p| {
            // Parse the repeat count
            let count = if let Ok(ident) = p.try_parse(|p| p.expect_ident().map(|s| s.to_string()))
            {
                match ident.as_str() {
                    "auto-fill" => types::GridRepeatCount::AutoFill,
                    "auto-fit" => types::GridRepeatCount::AutoFit,
                    _ => return Err(p.new_custom_error(())),
                }
            } else {
                let tok = p.next().cloned().map_err(|_| p.new_custom_error(()))?;
                let n = match tok {
                    Token::Number { int_value: Some(n), .. } if n > 0 => n as u16,
                    _ => return Err(p.new_custom_error(())),
                };
                types::GridRepeatCount::Count(n)
            };

            p.expect_comma().map_err(|_| p.new_custom_error(()))?;

            // Parse the track sizes inside repeat()
            let mut tracks = Vec::new();
            while !p.is_exhausted() {
                if let Ok(size) = p.try_parse(|p| parse_grid_track_size_single(p)) {
                    tracks.push(size);
                } else {
                    break;
                }
            }

            if tracks.is_empty() {
                return Err(p.new_custom_error(()));
            }

            Ok(types::GridTrackDef::Repeat(count, tracks))
        })
        .map_err(|_: cssparser::ParseError<'_, ()>| ())
}

/// Parse a grid placement value: `auto`, `<integer>`, `span <integer>`.
fn parse_grid_placement(parser: &mut Parser) -> Result<types::GridPlacement, ()> {
    // Try "auto"
    if let Ok(ident) = parser.try_parse(|p| p.expect_ident().map(|s| s.to_string())) {
        return match ident.as_str() {
            "auto" => Ok(types::GridPlacement::Auto),
            "span" => {
                // "span N"
                match parser.next().map_err(|_| ())? {
                    Token::Number { int_value: Some(n), .. } => {
                        Ok(types::GridPlacement::Span((*n).max(1) as u16))
                    }
                    _ => Err(()),
                }
            }
            _ => Err(()),
        };
    }

    // Try integer line number
    match parser.next().map_err(|_| ())? {
        Token::Number { int_value: Some(n), .. } => Ok(types::GridPlacement::Line(*n as i16)),
        _ => Err(()),
    }
}

pub fn apply_declaration(style: &mut ComputedStyle, decl: &StyleDeclaration) {
    match decl {
        StyleDeclaration::Content(v) => style.content = v.clone(),
        StyleDeclaration::Display(v) => style.display = *v,
        StyleDeclaration::FlexDirection(v) => style.flex_direction = *v,
        StyleDeclaration::FlexGrow(v) => style.flex_grow = *v,
        StyleDeclaration::FlexShrink(v) => style.flex_shrink = *v,
        StyleDeclaration::FlexBasis(v) => style.flex_basis = *v,
        StyleDeclaration::AlignItems(v) => style.align_items = *v,
        StyleDeclaration::AlignSelf(v) => style.align_self = *v,
        StyleDeclaration::JustifyContent(v) => style.justify_content = *v,
        StyleDeclaration::FlexWrap(v) => style.flex_wrap = *v,
        StyleDeclaration::AlignContent(v) => style.align_content = *v,
        StyleDeclaration::Width(v) => style.width = *v,
        StyleDeclaration::Height(v) => style.height = *v,
        StyleDeclaration::MinWidth(v) => style.min_width = *v,
        StyleDeclaration::MinHeight(v) => style.min_height = *v,
        StyleDeclaration::MaxWidth(v) => style.max_width = *v,
        StyleDeclaration::MaxHeight(v) => style.max_height = *v,
        StyleDeclaration::Padding(v) => style.padding = *v,
        StyleDeclaration::PaddingTop(v) => style.padding.top = *v,
        StyleDeclaration::PaddingRight(v) => style.padding.right = *v,
        StyleDeclaration::PaddingBottom(v) => style.padding.bottom = *v,
        StyleDeclaration::PaddingLeft(v) => style.padding.left = *v,
        StyleDeclaration::Margin(v) => style.margin = *v,
        StyleDeclaration::MarginTop(v) => style.margin.top = *v,
        StyleDeclaration::MarginRight(v) => style.margin.right = *v,
        StyleDeclaration::MarginBottom(v) => style.margin.bottom = *v,
        StyleDeclaration::MarginLeft(v) => style.margin.left = *v,
        StyleDeclaration::Gap(v) => {
            style.row_gap = *v;
            style.column_gap = *v;
        }
        StyleDeclaration::RowGap(v) => style.row_gap = *v,
        StyleDeclaration::ColumnGap(v) => style.column_gap = *v,
        StyleDeclaration::Overflow(v) => style.overflow = *v,
        StyleDeclaration::Background(v) => style.background = v.clone(),
        StyleDeclaration::BorderColor(v) => style.border_color = *v,
        StyleDeclaration::BorderWidth(v) => style.border_width = *v,
        StyleDeclaration::BorderRadius(v) => style.border_radius = *v,
        StyleDeclaration::Opacity(v) => style.opacity = *v,
        StyleDeclaration::BoxShadowList(v) => {
            let mut out: SmallVec<[types::BoxShadow; 2]> = SmallVec::with_capacity(v.len());
            for layer in v.iter() {
                // Default an omitted color to the element's current `color`
                // field, matching the CSS `currentColor` fallback. The
                // element defaults to opaque black when nothing else is set.
                let color = layer.color.unwrap_or(style.color);
                out.push(types::BoxShadow {
                    offset_x: layer.offset_x,
                    offset_y: layer.offset_y,
                    blur_radius: layer.blur_radius,
                    spread_radius: layer.spread_radius,
                    color,
                    inset: layer.inset,
                });
            }
            style.box_shadow = out;
        }
        StyleDeclaration::BackdropFilter(v) => style.backdrop_filter = Some(v.clone()),
        StyleDeclaration::Color(v) => style.color = *v,
        StyleDeclaration::FontSize(v) => style.font_size = *v,
        StyleDeclaration::FontWeight(v) => style.font_weight = *v,
        StyleDeclaration::FontFamily(v) => style.font_family = v.clone(),
        StyleDeclaration::LineHeight(v) => style.line_height = *v,
        StyleDeclaration::LetterSpacing(v) => style.letter_spacing = *v,
        StyleDeclaration::TextAlign(v) => style.text_align = *v,
        StyleDeclaration::TextDecoration(v) => style.text_decoration = *v,
        StyleDeclaration::TextDecorationColor(v) => style.text_decoration_color = Some(*v),
        StyleDeclaration::WhiteSpace(v) => style.white_space = *v,
        StyleDeclaration::Cursor(v) => style.cursor = *v,
        StyleDeclaration::Visibility(v) => style.visibility = *v,
        StyleDeclaration::PointerEvents(v) => style.pointer_events = *v,
        StyleDeclaration::UserSelect(v) => style.user_select = *v,
        StyleDeclaration::Position(v) => style.position = *v,
        StyleDeclaration::Top(v) => style.top = Some(*v),
        StyleDeclaration::Right(v) => style.right = Some(*v),
        StyleDeclaration::Bottom(v) => style.bottom = Some(*v),
        StyleDeclaration::Left(v) => style.left = Some(*v),
        StyleDeclaration::OutlineColor(v) => style.outline_color = *v,
        StyleDeclaration::OutlineWidth(v) => style.outline_width = *v,
        StyleDeclaration::OutlineOffset(v) => style.outline_offset = *v,
        StyleDeclaration::Layer(v) => style.layer = *v,
        StyleDeclaration::RenderTarget(v) => style.render_target = types::RenderTarget::Portal(*v),
        StyleDeclaration::CaretColor(v) => style.caret_color = *v,
        StyleDeclaration::CaretShape(v) => style.caret_shape = *v,
        StyleDeclaration::CaretBlinkRate(v) => style.caret_blink_rate = *v,
        StyleDeclaration::PlaceholderColor(v) => style.placeholder_color = *v,
        StyleDeclaration::Transition(v) => style.transitions = v.clone(),
        StyleDeclaration::Animation(v) => style.animations = v.clone(),
        StyleDeclaration::AnimationName(list) => {
            apply_animation_longhand(style, list.len(), |dst, i| {
                dst.name = list[i].clone();
            })
        }
        StyleDeclaration::AnimationDuration(list) => {
            apply_animation_longhand(style, list.len(), |dst, i| dst.duration = list[i])
        }
        StyleDeclaration::AnimationTimingFunction(list) => {
            apply_animation_longhand(style, list.len(), |dst, i| dst.timing_function = list[i])
        }
        StyleDeclaration::AnimationDelay(list) => {
            apply_animation_longhand(style, list.len(), |dst, i| {
                dst.delay = list[i].0;
                dst.delay_nanos = list[i].1;
            })
        }
        StyleDeclaration::AnimationIterationCount(list) => {
            apply_animation_longhand(style, list.len(), |dst, i| dst.iteration_count = list[i])
        }
        StyleDeclaration::AnimationDirection(list) => {
            apply_animation_longhand(style, list.len(), |dst, i| dst.direction = list[i])
        }
        StyleDeclaration::AnimationFillMode(list) => {
            apply_animation_longhand(style, list.len(), |dst, i| dst.fill_mode = list[i])
        }
        StyleDeclaration::AnimationPlayState(list) => {
            apply_animation_longhand(style, list.len(), |dst, i| dst.play_state = list[i])
        }

        // Keyboard capture
        StyleDeclaration::KeyboardCapture(v) => style.keyboard_capture = *v,

        // Grid container properties
        StyleDeclaration::GridTemplateColumns(v) => style.grid_template_columns = v.clone(),
        StyleDeclaration::GridTemplateRows(v) => style.grid_template_rows = v.clone(),
        StyleDeclaration::GridAutoColumns(v) => style.grid_auto_columns = v.clone(),
        StyleDeclaration::GridAutoRows(v) => style.grid_auto_rows = v.clone(),
        StyleDeclaration::GridAutoFlow(v) => style.grid_auto_flow = *v,

        // Grid item properties
        StyleDeclaration::GridColumnStart(v) => style.grid_column_start = *v,
        StyleDeclaration::GridColumnEnd(v) => style.grid_column_end = *v,
        StyleDeclaration::GridRowStart(v) => style.grid_row_start = *v,
        StyleDeclaration::GridRowEnd(v) => style.grid_row_end = *v,

        // Resize handle
        StyleDeclaration::ResizeAxis(v) => style.resize_axis = Some(*v),

        // Bell / notification
        StyleDeclaration::BellStyle(v) => style.bell_style = *v,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::transition::{TimingFunction, TransitionProperty};

    /// Helper: parse a CSS block and return the declarations for the first rule.
    fn parse_decls(css: &str) -> Vec<StyleDeclaration> {
        let sheet = CompiledStylesheet::parse(css);
        if sheet.rules.is_empty() {
            return vec![];
        }
        sheet.rules[0].declarations.clone()
    }

    #[test]
    fn test_transition_none() {
        let decls = parse_decls(".x { transition: none; }");
        let transition = decls.iter().find_map(|d| match d {
            StyleDeclaration::Transition(v) => Some(v),
            _ => None,
        });
        assert!(transition.is_some());
        assert!(transition.unwrap().is_empty());
    }

    #[test]
    fn test_transition_single_property() {
        let decls = parse_decls(".x { transition: opacity 0.3s ease; }");
        let defs = decls
            .iter()
            .find_map(|d| match d {
                StyleDeclaration::Transition(v) => Some(v),
                _ => None,
            })
            .expect("should have transition");

        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].property, TransitionProperty::Opacity);
        assert!((defs[0].duration.as_secs_f32() - 0.3).abs() < 0.01);
        assert_eq!(defs[0].timing_function, TimingFunction::Ease);
        assert_eq!(defs[0].delay, std::time::Duration::ZERO);
    }

    #[test]
    fn test_transition_with_ms_duration() {
        let decls = parse_decls(".x { transition: background 200ms linear; }");
        let defs = decls
            .iter()
            .find_map(|d| match d {
                StyleDeclaration::Transition(v) => Some(v),
                _ => None,
            })
            .expect("should have transition");

        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].property, TransitionProperty::Background);
        assert!((defs[0].duration.as_secs_f32() - 0.2).abs() < 0.01);
        assert_eq!(defs[0].timing_function, TimingFunction::Linear);
    }

    #[test]
    fn test_transition_with_delay() {
        let decls = parse_decls(".x { transition: opacity 0.3s ease-out 100ms; }");
        let defs = decls
            .iter()
            .find_map(|d| match d {
                StyleDeclaration::Transition(v) => Some(v),
                _ => None,
            })
            .expect("should have transition");

        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].timing_function, TimingFunction::EaseOut);
        assert!((defs[0].delay.as_secs_f32() - 0.1).abs() < 0.01);
    }

    #[test]
    fn test_transition_multiple_properties() {
        let decls =
            parse_decls(".x { transition: background 0.3s ease, opacity 0.2s ease-out 50ms; }");
        let defs = decls
            .iter()
            .find_map(|d| match d {
                StyleDeclaration::Transition(v) => Some(v),
                _ => None,
            })
            .expect("should have transition");

        assert_eq!(defs.len(), 2);

        assert_eq!(defs[0].property, TransitionProperty::Background);
        assert!((defs[0].duration.as_secs_f32() - 0.3).abs() < 0.01);
        assert_eq!(defs[0].timing_function, TimingFunction::Ease);

        assert_eq!(defs[1].property, TransitionProperty::Opacity);
        assert!((defs[1].duration.as_secs_f32() - 0.2).abs() < 0.01);
        assert_eq!(defs[1].timing_function, TimingFunction::EaseOut);
        assert!((defs[1].delay.as_secs_f32() - 0.05).abs() < 0.01);
    }

    #[test]
    fn test_transition_all_property() {
        let decls = parse_decls(".x { transition: all 0.5s ease-in-out; }");
        let defs = decls
            .iter()
            .find_map(|d| match d {
                StyleDeclaration::Transition(v) => Some(v),
                _ => None,
            })
            .expect("should have transition");

        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].property, TransitionProperty::All);
        assert_eq!(defs[0].timing_function, TimingFunction::EaseInOut);
    }

    #[test]
    fn test_transition_cubic_bezier() {
        let decls = parse_decls(".x { transition: all 0.5s cubic-bezier(0.4, 0, 0.2, 1); }");
        let defs = decls
            .iter()
            .find_map(|d| match d {
                StyleDeclaration::Transition(v) => Some(v),
                _ => None,
            })
            .expect("should have transition");

        assert_eq!(defs.len(), 1);
        match defs[0].timing_function {
            TimingFunction::CubicBezier(x1, y1, x2, y2) => {
                assert!((x1 - 0.4).abs() < 0.01);
                assert!((y1 - 0.0).abs() < 0.01);
                assert!((x2 - 0.2).abs() < 0.01);
                assert!((y2 - 1.0).abs() < 0.01);
            }
            other => panic!("expected CubicBezier, got {:?}", other),
        }
    }

    #[test]
    fn test_transition_default_timing_function() {
        // When no timing function is specified, defaults to ease.
        let decls = parse_decls(".x { transition: opacity 0.3s; }");
        let defs = decls
            .iter()
            .find_map(|d| match d {
                StyleDeclaration::Transition(v) => Some(v),
                _ => None,
            })
            .expect("should have transition");

        assert_eq!(defs[0].timing_function, TimingFunction::Ease);
    }

    #[test]
    fn test_transition_applied_to_computed_style() {
        let sheet = CompiledStylesheet::parse(
            ".btn { transition: opacity 0.3s ease, background 0.5s linear; }",
        );
        let mut style = ComputedStyle::default();
        for rule in &sheet.rules {
            for decl in &rule.declarations {
                apply_declaration(&mut style, decl);
            }
        }
        assert_eq!(style.transitions.len(), 2);
        assert_eq!(style.transitions[0].property, TransitionProperty::Opacity);
        assert_eq!(style.transitions[1].property, TransitionProperty::Background);
    }

    #[test]
    fn test_transition_ease_in_keyword() {
        let decls = parse_decls(".x { transition: color 1s ease-in; }");
        let defs = decls
            .iter()
            .find_map(|d| match d {
                StyleDeclaration::Transition(v) => Some(v),
                _ => None,
            })
            .expect("should have transition");
        assert_eq!(defs[0].timing_function, TimingFunction::EaseIn);
    }

    // ---------------------------------------------------------------------
    // Pseudo element and content property tests (see issue #121).
    // ---------------------------------------------------------------------

    fn last_parts_of(css: &str) -> Vec<SelectorPart> {
        let sheet = CompiledStylesheet::parse(css);
        assert!(!sheet.rules.is_empty(), "expected rule to parse: {css}");
        let chain = &sheet.rules[0].selector;
        chain.parts.last().expect("chain has parts").0.clone()
    }

    #[test]
    fn test_pseudo_element_before_parses() {
        let parts = last_parts_of("a::before { color: red; }");
        assert!(parts.iter().any(|p| matches!(p, SelectorPart::Tag(t) if t == "a")));
        assert!(parts
            .iter()
            .any(|p| matches!(p, SelectorPart::PseudoElement(PseudoElement::Before))));
    }

    #[test]
    fn test_pseudo_element_after_parses() {
        let parts = last_parts_of("a::after { color: blue; }");
        assert!(parts
            .iter()
            .any(|p| matches!(p, SelectorPart::PseudoElement(PseudoElement::After))));
    }

    #[test]
    fn test_pseudo_element_single_colon_legacy() {
        let before = last_parts_of("a:before { color: red; }");
        assert!(before
            .iter()
            .any(|p| matches!(p, SelectorPart::PseudoElement(PseudoElement::Before))));

        let after = last_parts_of("a:after { color: red; }");
        assert!(after
            .iter()
            .any(|p| matches!(p, SelectorPart::PseudoElement(PseudoElement::After))));
    }

    #[test]
    fn test_pseudo_element_with_class() {
        let parts = last_parts_of(".card::before { content: \"hi\"; }");
        let has_class = parts.iter().any(|p| matches!(p, SelectorPart::Class(c) if c == "card"));
        let has_before =
            parts.iter().any(|p| matches!(p, SelectorPart::PseudoElement(PseudoElement::Before)));
        assert!(has_class && has_before);
    }

    #[test]
    fn test_pseudo_element_with_pseudo_class() {
        let parts = last_parts_of("a:hover::before { content: \"!\"; }");
        // Must contain tag, hover pseudo class, and before pseudo element,
        // in this order in the flat selector part vector.
        let tag_pos =
            parts.iter().position(|p| matches!(p, SelectorPart::Tag(t) if t == "a")).expect("tag");
        let hover_pos = parts
            .iter()
            .position(|p| matches!(p, SelectorPart::PseudoClass(PseudoClass::Hover)))
            .expect("hover");
        let before_pos = parts
            .iter()
            .position(|p| matches!(p, SelectorPart::PseudoElement(PseudoElement::Before)))
            .expect("before");
        assert!(tag_pos < hover_pos && hover_pos < before_pos);
    }

    #[test]
    fn test_pseudo_element_selection_parses() {
        let parts = last_parts_of("p::selection { color: white; }");
        assert!(parts.iter().any(|p| matches!(p, SelectorPart::Tag(t) if t == "p")));
        assert!(parts
            .iter()
            .any(|p| matches!(p, SelectorPart::PseudoElement(PseudoElement::Selection))));
    }

    #[test]
    fn test_pseudo_element_moz_selection_parses() {
        let parts = last_parts_of("p::-moz-selection { color: white; }");
        assert!(parts
            .iter()
            .any(|p| matches!(p, SelectorPart::PseudoElement(PseudoElement::Selection))));
    }

    #[test]
    fn test_pseudo_element_selection_with_class() {
        let parts = last_parts_of(".highlight::selection { background-color: yellow; }");
        let has_class =
            parts.iter().any(|p| matches!(p, SelectorPart::Class(c) if c == "highlight"));
        let has_selection = parts
            .iter()
            .any(|p| matches!(p, SelectorPart::PseudoElement(PseudoElement::Selection)));
        assert!(has_class && has_selection);
    }

    #[test]
    fn test_inset_shorthand_expands_to_four_sides() {
        let decls = parse_decls(".x { inset: 10px; }");
        assert_eq!(decls.len(), 4);
        let px10 = Dimension::Px(10.0);
        assert!(matches!(&decls[0], StyleDeclaration::Top(d) if *d == px10));
        assert!(matches!(&decls[1], StyleDeclaration::Right(d) if *d == px10));
        assert!(matches!(&decls[2], StyleDeclaration::Bottom(d) if *d == px10));
        assert!(matches!(&decls[3], StyleDeclaration::Left(d) if *d == px10));
    }

    #[test]
    fn test_inset_zero() {
        let decls = parse_decls(".x { inset: 0; }");
        assert_eq!(decls.len(), 4);
        let zero = Dimension::Px(0.0);
        assert!(matches!(&decls[0], StyleDeclaration::Top(d) if *d == zero));
        assert!(matches!(&decls[1], StyleDeclaration::Right(d) if *d == zero));
        assert!(matches!(&decls[2], StyleDeclaration::Bottom(d) if *d == zero));
        assert!(matches!(&decls[3], StyleDeclaration::Left(d) if *d == zero));
    }

    #[test]
    fn test_vendor_pseudo_element_rejects_selector() {
        // Vendor-prefixed pseudo-elements like ::-webkit-scrollbar must cause
        // the entire selector to be rejected so that declarations inside the
        // rule are never applied to the base element.
        // Note: ::-moz-selection is NOT a vendor prefix; it is a recognized
        // alias for ::selection and parses successfully.
        let sheet = CompiledStylesheet::parse(
            r#"
            .pane-body::-webkit-scrollbar { width: 6px; }
            .pane-body::-webkit-scrollbar-thumb { background: #888; }
            .pane-body::-webkit-scrollbar-track { background: #f1f1f1; }
            "#,
        );
        assert_eq!(sheet.rules.len(), 0, "vendor pseudo-element rules must be discarded entirely");
    }

    #[test]
    fn test_vendor_pseudo_element_does_not_leak_to_host() {
        // Verify that a vendor pseudo-element rule does not leak its
        // declarations onto the base selector. Here .container should only
        // receive the color from the valid rule, not width from the scrollbar
        // rule.
        let sheet = CompiledStylesheet::parse(
            r#"
            .container { color: red; }
            .container::-webkit-scrollbar { width: 6px; }
            "#,
        );
        assert_eq!(sheet.rules.len(), 1, "only the valid rule should survive");
        assert!(sheet.rules[0]
            .selector
            .parts
            .last()
            .unwrap()
            .0
            .iter()
            .any(|p| matches!(p, SelectorPart::Class(c) if c == "container")));
    }

    #[test]
    fn test_before_after_still_work_alongside_vendor_pseudo() {
        // Ensure ::before and ::after continue to parse correctly even when
        // vendor pseudo-element rules appear in the same stylesheet.
        let sheet = CompiledStylesheet::parse(
            r#"
            .card::before { content: "!"; color: red; }
            .card::-webkit-scrollbar { width: 8px; }
            .card::after { content: "?"; color: blue; }
            "#,
        );
        assert_eq!(
            sheet.rules.len(),
            2,
            "before and after rules must survive, scrollbar must be discarded"
        );
        let pseudo_elements: Vec<_> =
            sheet.rules.iter().filter_map(|r| r.selector.pseudo_element()).collect();
        assert!(pseudo_elements.contains(&PseudoElement::Before));
        assert!(pseudo_elements.contains(&PseudoElement::After));
    }

    #[test]
    fn test_content_literal_string() {
        let decls = parse_decls(".x::before { content: \"hello\"; }");
        let v = decls.iter().find_map(|d| match d {
            StyleDeclaration::Content(v) => Some(v.clone()),
            _ => None,
        });
        assert_eq!(v, Some(ContentValue::Literal("hello".into())));
    }

    #[test]
    fn test_content_attr_ident() {
        let decls = parse_decls(".x::before { content: attr(id); }");
        let v = decls.iter().find_map(|d| match d {
            StyleDeclaration::Content(v) => Some(v.clone()),
            _ => None,
        });
        assert_eq!(v, Some(ContentValue::Attr("id".into())));
    }

    #[test]
    fn test_content_none_and_normal() {
        let none_decls = parse_decls(".x::before { content: none; }");
        assert!(none_decls
            .iter()
            .any(|d| matches!(d, StyleDeclaration::Content(ContentValue::None))));

        let normal_decls = parse_decls(".x::before { content: normal; }");
        assert!(normal_decls
            .iter()
            .any(|d| matches!(d, StyleDeclaration::Content(ContentValue::Normal))));
    }

    #[test]
    fn test_specificity_counts_pseudo_element() {
        // `.x::before` has one class (ab_b = 1) and one pseudo element counted
        // at the tag level (c = 1). Total specificity tuple = (0, 1, 1).
        let sheet = CompiledStylesheet::parse(".x::before { color: red; }");
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].specificity, (0, 1, 1));
    }

    // ------------------------------------------------------------------
    // @font-face parsing
    // ------------------------------------------------------------------

    #[test]
    fn test_font_face_url_rule() {
        let sheet = CompiledStylesheet::parse(
            "@font-face { font-family: \"Inter\"; src: url(\"inter.ttf\"); }",
        );
        assert_eq!(sheet.font_faces.len(), 1);
        assert_eq!(sheet.font_faces[0].family, "Inter");
        assert_eq!(sheet.font_faces[0].src, FontFaceSrc::Url("inter.ttf".to_string()));
    }

    #[test]
    fn test_font_face_local_rule() {
        let sheet = CompiledStylesheet::parse(
            "@font-face { font-family: \"SF Pro\"; src: local(\"Helvetica\"); }",
        );
        assert_eq!(sheet.font_faces.len(), 1);
        assert_eq!(sheet.font_faces[0].family, "SF Pro");
        assert_eq!(sheet.font_faces[0].src, FontFaceSrc::Local("Helvetica".to_string()));
    }

    #[test]
    fn test_font_face_unquoted_family() {
        let sheet = CompiledStylesheet::parse(
            "@font-face { font-family: Inter; src: url(\"inter.ttf\"); }",
        );
        assert_eq!(sheet.font_faces.len(), 1);
        assert_eq!(sheet.font_faces[0].family, "Inter");
    }

    #[test]
    fn test_font_face_multiple_rules() {
        let sheet = CompiledStylesheet::parse(
            "@font-face { font-family: \"A\"; src: url(\"a.ttf\"); } \
             @font-face { font-family: \"B\"; src: url(\"b.ttf\"); }",
        );
        assert_eq!(sheet.font_faces.len(), 2);
        assert_eq!(sheet.font_faces[0].family, "A");
        assert_eq!(sheet.font_faces[1].family, "B");
        assert_eq!(sheet.font_faces[0].src, FontFaceSrc::Url("a.ttf".to_string()));
        assert_eq!(sheet.font_faces[1].src, FontFaceSrc::Url("b.ttf".to_string()));
    }

    #[test]
    fn test_font_face_coexists_with_rules() {
        let sheet = CompiledStylesheet::parse(
            "body { color: red; } \
             @font-face { font-family: \"Inter\"; src: url(\"inter.ttf\"); } \
             .x { font-size: 16px; }",
        );
        assert_eq!(sheet.font_faces.len(), 1);
        assert_eq!(sheet.font_faces[0].family, "Inter");
        assert_eq!(sheet.rules.len(), 2);
    }

    #[test]
    fn test_unknown_at_rule_skipped() {
        // Both @media with a nested block and @charset (semicolon terminated)
        // should be skipped without breaking subsequent rules.
        let sheet = CompiledStylesheet::parse(
            "@charset \"UTF-8\"; \
             @media (min-width: 600px) { .x { color: blue; } } \
             .y { color: green; }",
        );
        // The .y rule should be the only compiled selector rule.
        assert!(
            sheet.rules.iter().any(|r| {
                r.selector.parts.iter().any(|(parts, _)| {
                    parts.iter().any(|p| matches!(p, SelectorPart::Class(c) if c == "y"))
                })
            }),
            "rules should still include .y, got {:?}",
            sheet.rules
        );
    }

    #[test]
    fn test_font_face_extra_descriptors_ignored() {
        let sheet = CompiledStylesheet::parse(
            "@font-face { \
                font-family: \"Inter\"; \
                font-weight: 400; \
                font-style: normal; \
                src: url(\"inter.ttf\"); \
                font-display: swap; \
             }",
        );
        assert_eq!(sheet.font_faces.len(), 1);
        assert_eq!(sheet.font_faces[0].family, "Inter");
        assert_eq!(sheet.font_faces[0].src, FontFaceSrc::Url("inter.ttf".to_string()));
    }

    #[test]
    fn test_font_face_missing_family_is_rejected() {
        // Missing font-family descriptor: the rule is dropped.
        let sheet = CompiledStylesheet::parse("@font-face { src: url(\"inter.ttf\"); }");
        assert!(sheet.font_faces.is_empty());
    }

    #[test]
    fn test_font_face_missing_src_is_rejected() {
        let sheet = CompiledStylesheet::parse("@font-face { font-family: \"Inter\"; }");
        assert!(sheet.font_faces.is_empty());
    }

    #[test]
    fn test_font_face_ordering_is_source_order() {
        let sheet = CompiledStylesheet::parse(
            "@font-face { font-family: \"C\"; src: url(\"c.ttf\"); } \
             @font-face { font-family: \"A\"; src: url(\"a.ttf\"); } \
             @font-face { font-family: \"B\"; src: url(\"b.ttf\"); }",
        );
        assert_eq!(sheet.font_faces.len(), 3);
        assert_eq!(sheet.font_faces[0].family, "C");
        assert_eq!(sheet.font_faces[1].family, "A");
        assert_eq!(sheet.font_faces[2].family, "B");
    }

    // -----------------------------------------------------------------------
    // linear-gradient parser (issue #126: N stop gradients)
    // -----------------------------------------------------------------------

    fn extract_gradient(css: &str) -> types::LinearGradient {
        let decls = parse_decls(css);
        decls
            .into_iter()
            .find_map(|d| match d {
                StyleDeclaration::Background(types::Background::LinearGradient(g)) => Some(g),
                _ => None,
            })
            .expect("expected a linear-gradient declaration")
    }

    fn approx_eq(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    /// Assert that a gradient stop position is `Percent(expected)`.
    #[track_caller]
    fn assert_percent(pos: types::GradientStopPosition, expected: f32) {
        match pos {
            types::GradientStopPosition::Percent(v) => {
                assert!(
                    approx_eq(v, expected),
                    "expected Percent({}), got Percent({})",
                    expected,
                    v
                );
            }
            types::GradientStopPosition::Px(v) => {
                panic!("expected Percent({}), got Px({})", expected, v)
            }
        }
    }

    /// Assert that a gradient stop position is `Px(expected)`.
    #[track_caller]
    fn assert_px(pos: types::GradientStopPosition, expected: f32) {
        match pos {
            types::GradientStopPosition::Px(v) => {
                assert!(approx_eq(v, expected), "expected Px({}), got Px({})", expected, v);
            }
            types::GradientStopPosition::Percent(v) => {
                panic!("expected Px({}), got Percent({})", expected, v)
            }
        }
    }

    #[test]
    fn test_linear_gradient_two_stops_with_explicit_positions() {
        // Reproduces a common terminal-manager site (theme chip).
        let g = extract_gradient(
            ".x { background: linear-gradient(135deg, #1e1a14 0%, #d4a348 100%); }",
        );
        assert!(approx_eq(g.angle_deg, 135.0));
        assert_eq!(g.stops.len(), 2);
        assert_percent(g.stops[0].position, 0.0);
        assert_percent(g.stops[1].position, 1.0);
        assert_eq!(g.stops[0].color.r, 0x1e);
        assert_eq!(g.stops[1].color.r, 0xd4);
        assert!(!g.repeating);
    }

    #[test]
    fn test_linear_gradient_two_stops_without_positions_defaults() {
        // Missing positions on both ends: first stop defaults to 0.0, last
        // to 1.0 per CSS Images Level 3 rule 1.
        let g = extract_gradient(".x { background: linear-gradient(90deg, red, blue); }");
        assert_eq!(g.stops.len(), 2);
        assert_percent(g.stops[0].position, 0.0);
        assert_percent(g.stops[1].position, 1.0);
        assert_eq!(g.stops[0].color, types::Color::rgb(255, 0, 0));
        assert_eq!(g.stops[1].color, types::Color::rgb(0, 0, 255));
    }

    #[test]
    fn test_linear_gradient_three_stops_shimmer() {
        // The shimmer pattern used across terminal-manager.
        let g = extract_gradient(
            ".x { background: linear-gradient(90deg, transparent, red 50%, transparent); }",
        );
        assert!(approx_eq(g.angle_deg, 90.0));
        assert_eq!(g.stops.len(), 3);
        assert_percent(g.stops[0].position, 0.0);
        assert_percent(g.stops[1].position, 0.5);
        assert_percent(g.stops[2].position, 1.0);
        assert_eq!(g.stops[0].color, types::Color::TRANSPARENT);
        assert_eq!(g.stops[1].color, types::Color::rgb(255, 0, 0));
        assert_eq!(g.stops[2].color, types::Color::TRANSPARENT);
    }

    #[test]
    fn test_linear_gradient_four_stops_vertical_separator() {
        // The 4 stop separator pattern: transparent, soft 20%, soft 80%,
        // transparent. All positions explicit.
        let g = extract_gradient(
            ".x { background: linear-gradient(180deg, transparent, #888888 20%, #888888 80%, transparent); }",
        );
        assert_eq!(g.stops.len(), 4);
        assert_percent(g.stops[0].position, 0.0);
        assert_percent(g.stops[1].position, 0.2);
        assert_percent(g.stops[2].position, 0.8);
        assert_percent(g.stops[3].position, 1.0);
    }

    #[test]
    fn test_linear_gradient_implicit_middle_positions_distributed_evenly() {
        // Four stops, only the endpoints positioned. The two interior
        // stops should be placed at 1/3 and 2/3 by rule 2.
        let g = extract_gradient(
            ".x { background: linear-gradient(90deg, red 0%, green, blue, black 100%); }",
        );
        assert_eq!(g.stops.len(), 4);
        assert_percent(g.stops[0].position, 0.0);
        assert_percent(g.stops[1].position, 1.0 / 3.0);
        assert_percent(g.stops[2].position, 2.0 / 3.0);
        assert_percent(g.stops[3].position, 1.0);
    }

    #[test]
    fn test_linear_gradient_single_stop_is_error() {
        // A single stop is not a valid gradient. The background property
        // falls back to solid color parsing, which also fails for this
        // input, so no Background declaration is emitted at all.
        let decls = parse_decls(".x { background: linear-gradient(90deg, red); }");
        let has_gradient = decls.iter().any(|d| {
            matches!(d, StyleDeclaration::Background(types::Background::LinearGradient(_)))
        });
        assert!(!has_gradient, "single stop gradient should not parse");
    }

    #[test]
    fn test_linear_gradient_non_monotonic_positions_clamped() {
        // rule 3: `blue 40%` is earlier than `red 60%`, so it must be
        // clamped up to 60% in the parsed output.
        let g = extract_gradient(".x { background: linear-gradient(90deg, red 60%, blue 40%); }");
        assert_eq!(g.stops.len(), 2);
        assert_percent(g.stops[0].position, 0.6);
        assert_percent(g.stops[1].position, 0.6);
    }

    #[test]
    fn test_linear_gradient_with_var_reference_resolves() {
        // var(--accent) in a stop color goes through the text level
        // resolver before the gradient parser runs, so by the time
        // parse_linear_gradient sees the tokens they already contain the
        // expanded value.
        let css = r#"
            :root { --accent: #ff00aa; }
            .x { background: linear-gradient(90deg, transparent, var(--accent) 50%, transparent); }
        "#;
        let sheet = CompiledStylesheet::parse(css);
        // :root has no class rule, skip it; look for the second rule.
        let rule = sheet
            .rules
            .iter()
            .find(|r| {
                !r.declarations.is_empty()
                    && matches!(
                        r.declarations.first(),
                        Some(StyleDeclaration::Background(types::Background::LinearGradient(_)))
                    )
            })
            .expect("expected a gradient rule");
        let g = match rule.declarations.first() {
            Some(StyleDeclaration::Background(types::Background::LinearGradient(g))) => g,
            _ => panic!("expected LinearGradient"),
        };
        assert_eq!(g.stops.len(), 3);
        assert_eq!(g.stops[1].color, types::Color::rgba(0xff, 0x00, 0xaa, 0xff));
    }

    #[test]
    fn test_linear_gradient_transparent_keyword_at_boundaries() {
        // Both endpoints use the `transparent` keyword. Verifies it
        // resolves to Color::TRANSPARENT (alpha 0) and does not throw off
        // the position fixup pass.
        let g = extract_gradient(
            ".x { background: linear-gradient(90deg, transparent 0%, black 50%, transparent 100%); }",
        );
        assert_eq!(g.stops.len(), 3);
        assert_eq!(g.stops[0].color, types::Color::TRANSPARENT);
        assert_eq!(g.stops[0].color.a, 0);
        assert_eq!(g.stops[2].color, types::Color::TRANSPARENT);
        assert_eq!(g.stops[2].color.a, 0);
        assert_eq!(g.stops[1].color.a, 255);
    }

    #[test]
    fn test_background_is_visible_all_transparent_stops() {
        // A gradient where every stop is transparent should not be
        // considered visible (nothing to draw).
        let g = types::LinearGradient {
            angle_deg: 0.0,
            repeating: false,
            stops: smallvec::smallvec![
                types::GradientStop {
                    color: types::Color::TRANSPARENT,
                    position: types::GradientStopPosition::Percent(0.0),
                },
                types::GradientStop {
                    color: types::Color::TRANSPARENT,
                    position: types::GradientStopPosition::Percent(0.5),
                },
                types::GradientStop {
                    color: types::Color::TRANSPARENT,
                    position: types::GradientStopPosition::Percent(1.0),
                },
            ],
        };
        let bg = types::Background::LinearGradient(g);
        assert!(!bg.is_visible());
    }

    #[test]
    fn test_background_is_visible_with_one_opaque_inner_stop() {
        // At least one non transparent stop makes the gradient visible.
        let g = types::LinearGradient {
            angle_deg: 90.0,
            repeating: false,
            stops: smallvec::smallvec![
                types::GradientStop {
                    color: types::Color::TRANSPARENT,
                    position: types::GradientStopPosition::Percent(0.0),
                },
                types::GradientStop {
                    color: types::Color::rgb(255, 0, 0),
                    position: types::GradientStopPosition::Percent(0.5),
                },
                types::GradientStop {
                    color: types::Color::TRANSPARENT,
                    position: types::GradientStopPosition::Percent(1.0),
                },
            ],
        };
        let bg = types::Background::LinearGradient(g);
        assert!(bg.is_visible());
    }

    #[test]
    fn test_linear_gradient_default_angle_is_180_deg() {
        // Without an explicit angle the gradient flows top to bottom
        // (180deg), matching the CSS default.
        let g = extract_gradient(".x { background: linear-gradient(red, blue); }");
        assert!(approx_eq(g.angle_deg, 180.0));
        assert_eq!(g.stops.len(), 2);
        assert!(!g.repeating);
    }

    // -----------------------------------------------------------------------
    // repeating-linear-gradient parser (issue #128)
    // -----------------------------------------------------------------------

    #[test]
    fn test_repeating_linear_gradient_pixel_stops() {
        // The exact terminal-manager scanline tile: a 3 pixel pattern with
        // two transparent rows followed by a one pixel translucent black
        // row, repeated along the vertical axis (0deg in CSS).
        let g = extract_gradient(
            ".x { background: repeating-linear-gradient(0deg, red 0, red 2px, blue 2px, blue 3px); }",
        );
        assert!(g.repeating);
        assert!(approx_eq(g.angle_deg, 0.0));
        assert_eq!(g.stops.len(), 4);
        assert_px(g.stops[0].position, 0.0);
        assert_px(g.stops[1].position, 2.0);
        assert_px(g.stops[2].position, 2.0);
        assert_px(g.stops[3].position, 3.0);
    }

    #[test]
    fn test_repeating_linear_gradient_no_angle_no_positions() {
        // Without an explicit angle or positions the parser must still
        // succeed. Default angle is 180deg, default positions are 0% and
        // 100%, and the repeating flag must be true.
        let g = extract_gradient(".x { background: repeating-linear-gradient(red, blue); }");
        assert!(g.repeating);
        assert!(approx_eq(g.angle_deg, 180.0));
        assert_eq!(g.stops.len(), 2);
        assert_percent(g.stops[0].position, 0.0);
        assert_percent(g.stops[1].position, 1.0);
    }

    #[test]
    fn test_linear_gradient_keeps_repeating_false() {
        // Sanity check that the non repeating function still parses with
        // `repeating == false` after the flag landed.
        let g = extract_gradient(".x { background: linear-gradient(0deg, red 0, blue 100%); }");
        assert!(!g.repeating);
        assert_eq!(g.stops.len(), 2);
        assert_px(g.stops[0].position, 0.0);
        assert_percent(g.stops[1].position, 1.0);
    }

    #[test]
    fn test_repeating_linear_gradient_single_stop_is_error() {
        // A single stop is invalid for both linear and repeating linear
        // gradients. The dispatcher rejects it because it cannot fall
        // through to a solid color either.
        let decls = parse_decls(".x { background: repeating-linear-gradient(0deg, red); }");
        let has_gradient = decls.iter().any(|d| {
            matches!(d, StyleDeclaration::Background(types::Background::LinearGradient(_)))
        });
        assert!(!has_gradient, "single stop repeating gradient should not parse");
    }

    #[test]
    fn test_repeating_linear_gradient_zero_tile_is_error() {
        // First and last stops at the same position would make the tile
        // span zero, causing a divide by near zero in the shader.
        let decls =
            parse_decls(".x { background: repeating-linear-gradient(0deg, red 2px, blue 2px); }");
        let has_gradient = decls.iter().any(|d| {
            matches!(d, StyleDeclaration::Background(types::Background::LinearGradient(_)))
        });
        assert!(!has_gradient, "zero tile repeating gradient should not parse");
    }

    #[test]
    fn test_repeating_linear_gradient_terminal_manager_scanline() {
        // Round trip the exact string at terminal-manager/styles.css line
        // 169 through the parser. This is the ground truth string the
        // CRT scanline overlay uses, and the resolver test below depends
        // on it parsing without quoting tweaks.
        let g = extract_gradient(
            ".x { background-image: repeating-linear-gradient(0deg, transparent 0, transparent 2px, rgba(0,0,0,0.12) 2px, rgba(0,0,0,0.12) 3px); }",
        );
        assert!(g.repeating);
        assert!(approx_eq(g.angle_deg, 0.0));
        assert_eq!(g.stops.len(), 4);
        assert_px(g.stops[0].position, 0.0);
        assert_px(g.stops[1].position, 2.0);
        assert_px(g.stops[2].position, 2.0);
        assert_px(g.stops[3].position, 3.0);
        assert_eq!(g.stops[0].color, types::Color::TRANSPARENT);
        assert_eq!(g.stops[1].color, types::Color::TRANSPARENT);
        // rgba(0,0,0,0.12) parses to alpha = round(0.12 * 255) = 31.
        assert_eq!(g.stops[2].color.r, 0);
        assert_eq!(g.stops[2].color.g, 0);
        assert_eq!(g.stops[2].color.b, 0);
        assert!(g.stops[2].color.a >= 30 && g.stops[2].color.a <= 32);
        assert_eq!(g.stops[3].color.r, 0);
        assert!(g.stops[3].color.a >= 30 && g.stops[3].color.a <= 32);
    }

    #[test]
    fn test_repeating_linear_gradient_position_resolve() {
        // Verify that the GradientStopPosition resolver normalizes pixel
        // values against the projected axis length, with a 600 pixel axis
        // turning a 3 pixel tile into 0.005 in normalized space.
        let pos_first = types::GradientStopPosition::Px(0.0);
        let pos_last = types::GradientStopPosition::Px(3.0);
        let axis_length: f32 = 600.0;
        let first = pos_first.resolve(axis_length);
        let last = pos_last.resolve(axis_length);
        assert!(approx_eq(first, 0.0));
        assert!(approx_eq(last, 3.0 / 600.0));
        // Negative pixel positions clamp to zero per the CSS spec.
        let neg = types::GradientStopPosition::Px(-5.0);
        assert!(approx_eq(neg.resolve(axis_length), 0.0));
        // Percent positions pass through unchanged.
        let pct = types::GradientStopPosition::Percent(0.5);
        assert!(approx_eq(pct.resolve(axis_length), 0.5));
    }

    #[test]
    fn test_linear_gradient_with_pixel_positions_non_repeating() {
        // Pixel positions on a non repeating gradient also work; the
        // resolver still normalizes them at batch time.
        let g = extract_gradient(".x { background: linear-gradient(180deg, red 0px, blue 50px); }");
        assert!(!g.repeating);
        assert_eq!(g.stops.len(), 2);
        assert_px(g.stops[0].position, 0.0);
        assert_px(g.stops[1].position, 50.0);
    }

    #[test]
    fn test_linear_gradient_negative_position_clamped_to_zero() {
        // CSS spec: stop positions are clamped to be non negative. The
        // monotonic clamp pass enforces this since positions start from
        // a previous floor of 0.0.
        let g = extract_gradient(".x { background: linear-gradient(0deg, red -10%, blue 100%); }");
        assert_percent(g.stops[0].position, 0.0);
        assert_percent(g.stops[1].position, 1.0);
    }

    // -----------------------------------------------------------------------
    // radial-gradient parser (issue #127)
    // -----------------------------------------------------------------------

    fn extract_radial(css: &str) -> types::RadialGradient {
        let decls = parse_decls(css);
        decls
            .into_iter()
            .find_map(|d| match d {
                StyleDeclaration::Background(types::Background::RadialGradient(g)) => Some(g),
                _ => None,
            })
            .expect("expected a radial-gradient declaration")
    }

    fn lp_percent(v: f32) -> types::LengthOrPercent {
        types::LengthOrPercent::Percent(v)
    }

    #[test]
    fn radial_gradient_default_shape_is_ellipse() {
        // No explicit shape: CSS defaults to ellipse, farthest corner,
        // centered.
        let g = extract_radial(".x { background: radial-gradient(red, blue); }");
        assert_eq!(g.shape, types::RadialShape::Ellipse);
        assert_eq!(g.size, types::RadialSize::FarthestCorner);
        assert_eq!(g.center, types::RadialPosition::CENTER);
        assert_eq!(g.stops.len(), 2);
    }

    #[test]
    fn radial_gradient_circle() {
        // Explicit `circle` keyword sets the shape to Circle. With no size
        // value, the size defaults to farthest corner.
        let g = extract_radial(".x { background: radial-gradient(circle, red, blue); }");
        assert_eq!(g.shape, types::RadialShape::Circle);
        assert_eq!(g.size, types::RadialSize::FarthestCorner);
    }

    #[test]
    fn radial_gradient_explicit_size_two_percentages() {
        // Two percentages: explicit ellipse radii in width/height fractions.
        let g = extract_radial(".x { background: radial-gradient(60% 40%, red, blue); }");
        assert_eq!(g.shape, types::RadialShape::Ellipse);
        match g.size {
            types::RadialSize::Explicit { rx, ry } => {
                assert_eq!(rx, lp_percent(0.6));
                assert_eq!(ry, lp_percent(0.4));
            }
            _ => panic!("expected explicit size, got {:?}", g.size),
        }
    }

    #[test]
    fn radial_gradient_explicit_size_one_length_circle() {
        // A single length value implies a circle when no shape was given.
        let g = extract_radial(".x { background: radial-gradient(50px, red, blue); }");
        assert_eq!(g.shape, types::RadialShape::Circle);
        match g.size {
            types::RadialSize::Explicit { rx, ry } => {
                assert_eq!(rx, types::LengthOrPercent::Px(50.0));
                assert_eq!(ry, types::LengthOrPercent::Px(50.0));
            }
            _ => panic!("expected explicit size, got {:?}", g.size),
        }
    }

    #[test]
    fn radial_gradient_keyword_size_closest_side() {
        let g = extract_radial(".x { background: radial-gradient(closest-side, red, blue); }");
        assert_eq!(g.size, types::RadialSize::ClosestSide);
    }

    #[test]
    fn radial_gradient_keyword_size_farthest_corner() {
        let g = extract_radial(
            ".x { background: radial-gradient(ellipse farthest-corner, red, blue); }",
        );
        assert_eq!(g.shape, types::RadialShape::Ellipse);
        assert_eq!(g.size, types::RadialSize::FarthestCorner);
    }

    #[test]
    fn radial_gradient_position_two_keywords() {
        // `at top left` resolves to (0%, 0%) regardless of order.
        let g = extract_radial(".x { background: radial-gradient(at top left, red, blue); }");
        assert_eq!(g.center, types::RadialPosition { x: lp_percent(0.0), y: lp_percent(0.0) },);
    }

    #[test]
    fn radial_gradient_position_percent_pair() {
        let g = extract_radial(".x { background: radial-gradient(at 25% 75%, red, blue); }");
        assert_eq!(g.center, types::RadialPosition { x: lp_percent(0.25), y: lp_percent(0.75) },);
    }

    #[test]
    fn radial_gradient_position_at_top() {
        // `at top` matches `terminal-manager/styles.css:948`. Without an x
        // axis component, x defaults to center (50%) and y becomes 0.
        let g = extract_radial(".x { background: radial-gradient(ellipse at top, red, blue); }");
        assert_eq!(g.center, types::RadialPosition { x: lp_percent(0.5), y: lp_percent(0.0) },);
    }

    #[test]
    fn radial_gradient_two_stops() {
        // Default position fixup: first stop becomes 0.0, last becomes 1.0.
        let g = extract_radial(".x { background: radial-gradient(red, blue); }");
        assert_eq!(g.stops.len(), 2);
        assert!(approx_eq(g.stops[0].position.resolve(1.0), 0.0));
        assert!(approx_eq(g.stops[1].position.resolve(1.0), 1.0));
        assert_eq!(g.stops[0].color, types::Color::rgb(255, 0, 0));
        assert_eq!(g.stops[1].color, types::Color::rgb(0, 0, 255));
    }

    #[test]
    fn radial_gradient_three_stops() {
        // Three stops with explicit middle position.
        let g = extract_radial(
            ".x { background: radial-gradient(transparent, red 50%, transparent); }",
        );
        assert_eq!(g.stops.len(), 3);
        assert!(approx_eq(g.stops[0].position.resolve(1.0), 0.0));
        assert!(approx_eq(g.stops[1].position.resolve(1.0), 0.5));
        assert!(approx_eq(g.stops[2].position.resolve(1.0), 1.0));
        assert_eq!(g.stops[1].color, types::Color::rgb(255, 0, 0));
    }

    #[test]
    fn radial_gradient_terminal_manager_body_before_layer_one() {
        // Verbatim line 158 of terminal-manager/styles.css.
        let g = extract_radial(
            ".x { background: radial-gradient(ellipse 60% 40% at 85% 0%, rgba(212, 163, 72, 0.035), transparent 70%); }",
        );
        assert_eq!(g.shape, types::RadialShape::Ellipse);
        match g.size {
            types::RadialSize::Explicit { rx, ry } => {
                assert_eq!(rx, lp_percent(0.6));
                assert_eq!(ry, lp_percent(0.4));
            }
            _ => panic!("expected explicit size"),
        }
        assert_eq!(g.center, types::RadialPosition { x: lp_percent(0.85), y: lp_percent(0.0) },);
        assert_eq!(g.stops.len(), 2);
        // First stop is the warm glow with very low alpha.
        assert_eq!(g.stops[0].color.r, 212);
        assert_eq!(g.stops[0].color.g, 163);
        assert_eq!(g.stops[0].color.b, 72);
        assert!(g.stops[0].color.a > 0 && g.stops[0].color.a < 16);
        // Second stop is transparent with explicit position 70%.
        assert_eq!(g.stops[1].color.a, 0);
        assert!(approx_eq(g.stops[1].position.resolve(1.0), 0.7));
    }

    #[test]
    fn radial_gradient_terminal_manager_body_before_layer_two() {
        // Verbatim line 159 of terminal-manager/styles.css.
        let g = extract_radial(
            ".x { background: radial-gradient(ellipse 80% 50% at 10% 100%, rgba(138, 96, 32, 0.025), transparent 70%); }",
        );
        match g.size {
            types::RadialSize::Explicit { rx, ry } => {
                assert_eq!(rx, lp_percent(0.8));
                assert_eq!(ry, lp_percent(0.5));
            }
            _ => panic!("expected explicit size"),
        }
        assert_eq!(g.center, types::RadialPosition { x: lp_percent(0.1), y: lp_percent(1.0) },);
        assert_eq!(g.stops[0].color.r, 138);
    }

    #[test]
    fn radial_gradient_terminal_manager_pane_before() {
        // Verbatim line 948 of terminal-manager/styles.css. Default size,
        // position keyword, two stops.
        let g = extract_radial(
            ".x { background: radial-gradient(ellipse at top, rgba(212, 163, 72, 0.015), transparent 60%); }",
        );
        assert_eq!(g.shape, types::RadialShape::Ellipse);
        assert_eq!(g.size, types::RadialSize::FarthestCorner);
        assert_eq!(g.center, types::RadialPosition { x: lp_percent(0.5), y: lp_percent(0.0) },);
        assert_eq!(g.stops[0].color.r, 212);
        assert!(approx_eq(g.stops[1].position.resolve(1.0), 0.6));
    }

    #[test]
    fn radial_gradient_single_stop_is_error() {
        // CSS requires at least two color stops. A single stop should not
        // produce a gradient declaration; the parser falls back to the
        // color path which also fails for `radial-gradient(...)`, so no
        // background declaration is emitted at all.
        let decls = parse_decls(".x { background: radial-gradient(red); }");
        let has_radial = decls.iter().any(|d| {
            matches!(d, StyleDeclaration::Background(types::Background::RadialGradient(_)))
        });
        assert!(!has_radial, "single stop radial gradient should not parse");
    }

    #[test]
    fn radial_gradient_negative_explicit_size_is_error() {
        // Negative explicit radii are rejected at parse time per CSS spec.
        let decls = parse_decls(".x { background: radial-gradient(-50% 40%, red, blue); }");
        let has_radial = decls.iter().any(|d| {
            matches!(d, StyleDeclaration::Background(types::Background::RadialGradient(_)))
        });
        assert!(!has_radial, "negative size should be rejected");
    }

    #[test]
    fn radial_gradient_background_is_visible() {
        // A radial with at least one opaque stop counts as visible.
        let g = types::RadialGradient {
            shape: types::RadialShape::Ellipse,
            size: types::RadialSize::FarthestCorner,
            center: types::RadialPosition::CENTER,
            stops: smallvec::smallvec![
                types::GradientStop {
                    color: types::Color::rgb(255, 0, 0),
                    position: types::GradientStopPosition::Percent(0.0)
                },
                types::GradientStop {
                    color: types::Color::TRANSPARENT,
                    position: types::GradientStopPosition::Percent(1.0)
                },
            ],
        };
        let bg = types::Background::RadialGradient(g);
        assert!(bg.is_visible());
    }

    #[test]
    fn radial_gradient_background_invisible_when_all_transparent() {
        let g = types::RadialGradient {
            shape: types::RadialShape::Ellipse,
            size: types::RadialSize::FarthestCorner,
            center: types::RadialPosition::CENTER,
            stops: smallvec::smallvec![
                types::GradientStop {
                    color: types::Color::TRANSPARENT,
                    position: types::GradientStopPosition::Percent(0.0)
                },
                types::GradientStop {
                    color: types::Color::TRANSPARENT,
                    position: types::GradientStopPosition::Percent(1.0)
                },
            ],
        };
        let bg = types::Background::RadialGradient(g);
        assert!(!bg.is_visible());
    }

    // -----------------------------------------------------------------------
    // RadialGradient resolver (size keyword resolution at paint time)
    // -----------------------------------------------------------------------

    fn radial_with(
        shape: types::RadialShape,
        size: types::RadialSize,
        cx: f32,
        cy: f32,
    ) -> types::RadialGradient {
        types::RadialGradient {
            shape,
            size,
            center: types::RadialPosition { x: lp_percent(cx), y: lp_percent(cy) },
            stops: smallvec::smallvec![
                types::GradientStop {
                    color: types::Color::TRANSPARENT,
                    position: types::GradientStopPosition::Percent(0.0)
                },
                types::GradientStop {
                    color: types::Color::WHITE,
                    position: types::GradientStopPosition::Percent(1.0)
                },
            ],
        }
    }

    #[test]
    fn closest_side_ellipse_centered() {
        // Centered in a 200x100 box: closest-side ellipse should hug the
        // shortest pair of sides on each axis: rx = 100, ry = 50.
        let g = radial_with(types::RadialShape::Ellipse, types::RadialSize::ClosestSide, 0.5, 0.5);
        let r = g.resolve(200.0, 100.0);
        assert!(approx_eq(r.center_x, 100.0));
        assert!(approx_eq(r.center_y, 50.0));
        assert!(approx_eq(r.rx, 100.0));
        assert!(approx_eq(r.ry, 50.0));
    }

    #[test]
    fn closest_side_ellipse_off_center() {
        // Center at (40, 25) inside 200x100. Closest x side is 40, closest
        // y side is 25.
        let g = radial_with(types::RadialShape::Ellipse, types::RadialSize::ClosestSide, 0.2, 0.25);
        let r = g.resolve(200.0, 100.0);
        assert!(approx_eq(r.rx, 40.0));
        assert!(approx_eq(r.ry, 25.0));
    }

    #[test]
    fn farthest_corner_default_centered() {
        // 200x100 centered: farthest corner is at (200, 100). Distance
        // from (100, 50) is hypot(100, 50). Ellipse scales by sqrt(2).
        let g =
            radial_with(types::RadialShape::Ellipse, types::RadialSize::FarthestCorner, 0.5, 0.5);
        let r = g.resolve(200.0, 100.0);
        let k = std::f32::consts::SQRT_2;
        assert!(approx_eq(r.rx, 100.0 * k));
        assert!(approx_eq(r.ry, 50.0 * k));
    }

    #[test]
    fn farthest_corner_off_center() {
        // Center at (10, 10) inside 100x100. Farthest x side is 90,
        // farthest y side is 90.
        let g =
            radial_with(types::RadialShape::Ellipse, types::RadialSize::FarthestCorner, 0.1, 0.1);
        let r = g.resolve(100.0, 100.0);
        let k = std::f32::consts::SQRT_2;
        assert!(approx_eq(r.rx, 90.0 * k));
        assert!(approx_eq(r.ry, 90.0 * k));
    }

    #[test]
    fn closest_corner_circle() {
        // Circle in 100x100 centered: closest corner is hypot(50, 50).
        let g = radial_with(types::RadialShape::Circle, types::RadialSize::ClosestCorner, 0.5, 0.5);
        let r = g.resolve(100.0, 100.0);
        let expected = (50.0_f32 * 50.0 + 50.0_f32 * 50.0).sqrt();
        assert!(approx_eq(r.rx, expected));
        assert!(approx_eq(r.ry, expected));
    }

    #[test]
    fn explicit_percent_resolves_against_width_and_height() {
        // 60% of width is 120, 40% of height is 40.
        let g = types::RadialGradient {
            shape: types::RadialShape::Ellipse,
            size: types::RadialSize::Explicit { rx: lp_percent(0.6), ry: lp_percent(0.4) },
            center: types::RadialPosition::CENTER,
            stops: smallvec::smallvec![
                types::GradientStop {
                    color: types::Color::TRANSPARENT,
                    position: types::GradientStopPosition::Percent(0.0)
                },
                types::GradientStop {
                    color: types::Color::WHITE,
                    position: types::GradientStopPosition::Percent(1.0)
                },
            ],
        };
        let r = g.resolve(200.0, 100.0);
        assert!(approx_eq(r.rx, 120.0));
        assert!(approx_eq(r.ry, 40.0));
    }

    // ---- box-shadow parsing tests ----

    fn get_box_shadow_list(decls: &[StyleDeclaration]) -> SmallVec<[ParsedBoxShadow; 2]> {
        decls
            .iter()
            .find_map(|d| match d {
                StyleDeclaration::BoxShadowList(v) => Some(v.clone()),
                _ => None,
            })
            .expect("declaration should contain a box-shadow list")
    }

    #[test]
    fn box_shadow_single_outer() {
        let decls = parse_decls(".x { box-shadow: 0 0 10px rgba(212,163,72,0.6); }");
        let list = get_box_shadow_list(&decls);
        assert_eq!(list.len(), 1);
        let s = list[0];
        assert_eq!(s.offset_x, 0.0);
        assert_eq!(s.offset_y, 0.0);
        assert_eq!(s.blur_radius, 10.0);
        assert_eq!(s.spread_radius, 0.0);
        assert!(!s.inset);
        let c = s.color.expect("explicit color should be parsed");
        assert_eq!((c.r, c.g, c.b), (212, 163, 72));
        assert!(c.a >= 150 && c.a <= 155);
    }

    #[test]
    fn box_shadow_single_inset() {
        let decls = parse_decls(".x { box-shadow: inset 0 0 6px rgba(212,163,72,0.3); }");
        let list = get_box_shadow_list(&decls);
        assert_eq!(list.len(), 1);
        let s = list[0];
        assert_eq!(s.blur_radius, 6.0);
        assert!(s.inset);
        assert!(s.color.is_some());
    }

    #[test]
    fn box_shadow_inset_trailing() {
        let decls = parse_decls(".x { box-shadow: 0 0 6px rgba(0,0,0,0.5) inset; }");
        let list = get_box_shadow_list(&decls);
        assert_eq!(list.len(), 1);
        assert!(list[0].inset);
        assert_eq!(list[0].blur_radius, 6.0);
    }

    #[test]
    fn box_shadow_stacked_outer_inset() {
        // terminal-manager line 953 ground truth.
        let decls = parse_decls(
            ".x { box-shadow: inset 0 0 0 1px rgba(212,163,72,0.12), 0 0 20px rgba(0,0,0,0.25); }",
        );
        let list = get_box_shadow_list(&decls);
        assert_eq!(list.len(), 2);
        assert!(list[0].inset);
        assert_eq!(list[0].spread_radius, 1.0);
        assert_eq!(list[0].blur_radius, 0.0);
        assert!(!list[1].inset);
        assert_eq!(list[1].blur_radius, 20.0);
    }

    #[test]
    fn box_shadow_three_outer() {
        // terminal-manager line 1304 ground truth (var(--shadow-lg) has been
        // manually expanded here to the outer shadow it resolves to).
        let decls = parse_decls(
            ".x { box-shadow: 0 14px 40px rgba(0,0,0,0.65), 0 0 0 1px rgba(212,163,72,0.08), 0 0 60px rgba(212,163,72,0.06); }",
        );
        let list = get_box_shadow_list(&decls);
        assert_eq!(list.len(), 3);
        assert!(list.iter().all(|s| !s.inset));
        assert_eq!(list[0].offset_y, 14.0);
        assert_eq!(list[0].blur_radius, 40.0);
        assert_eq!(list[1].spread_radius, 1.0);
        assert_eq!(list[2].blur_radius, 60.0);
    }

    #[test]
    fn box_shadow_none() {
        let decls = parse_decls(".x { box-shadow: none; }");
        let list = get_box_shadow_list(&decls);
        assert!(list.is_empty());
    }

    #[test]
    fn box_shadow_default_color() {
        let decls = parse_decls(".x { box-shadow: 0 0 6px; }");
        let list = get_box_shadow_list(&decls);
        assert_eq!(list.len(), 1);
        // No color provided: the parser leaves it `None` so the resolver
        // can fall back to `currentColor` at apply time.
        assert!(list[0].color.is_none());
        assert_eq!(list[0].blur_radius, 6.0);
    }

    #[test]
    fn box_shadow_zero_blur_with_spread() {
        let decls = parse_decls(".x { box-shadow: 0 0 0 2px rgba(212,163,72,0.12); }");
        let list = get_box_shadow_list(&decls);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].blur_radius, 0.0);
        assert_eq!(list[0].spread_radius, 2.0);
        assert!(list[0].color.is_some());
    }

    #[test]
    fn box_shadow_resolver_writes_full_list() {
        // Stacked two-layer declaration should land in ComputedStyle as a
        // two-element vec with colors resolved.
        let decls = parse_decls(
            ".x { color: #ff0000; box-shadow: inset 0 0 0 1px rgba(212,163,72,0.12), 0 0 20px rgba(0,0,0,0.25); }",
        );
        let mut style = ComputedStyle::default();
        for d in &decls {
            apply_declaration(&mut style, d);
        }
        assert_eq!(style.box_shadow.len(), 2);
        assert!(style.box_shadow[0].inset);
        assert_eq!(style.box_shadow[0].spread_radius, 1.0);
        assert!(!style.box_shadow[1].inset);
        assert_eq!(style.box_shadow[1].blur_radius, 20.0);
    }

    #[test]
    fn box_shadow_resolver_default_color_uses_element_color() {
        // `color` declared before `box-shadow` with missing shadow color:
        // the resolver should fill the shadow color with the element's
        // current `color`.
        let decls = parse_decls(".x { color: #112233; box-shadow: 0 0 6px; }");
        let mut style = ComputedStyle::default();
        for d in &decls {
            apply_declaration(&mut style, d);
        }
        assert_eq!(style.box_shadow.len(), 1);
        let c = style.box_shadow[0].color;
        assert_eq!((c.r, c.g, c.b), (0x11, 0x22, 0x33));
    }

    // ------------------------------------------------------------------
    // @keyframes parsing (issue #129)
    // ------------------------------------------------------------------

    #[test]
    fn test_parse_keyframes_from_to() {
        let sheet = CompiledStylesheet::parse(
            "@keyframes fade-in { from { opacity: 0; } to { opacity: 1; } }",
        );
        let rule = sheet.keyframes.get("fade-in").expect("fade-in registered");
        assert_eq!(rule.frames.len(), 2);
        assert!((rule.frames[0].offset - 0.0).abs() < 1e-6);
        assert!((rule.frames[1].offset - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_parse_keyframes_percentages() {
        let sheet = CompiledStylesheet::parse(
            "@keyframes pulse-dot { 0%, 100% { opacity: 1; } 50% { opacity: 0.4; } }",
        );
        let rule = sheet.keyframes.get("pulse-dot").expect("pulse-dot registered");
        assert_eq!(rule.frames.len(), 3);
        // Frames are sorted by offset.
        assert!((rule.frames[0].offset - 0.0).abs() < 1e-6);
        assert!((rule.frames[1].offset - 0.5).abs() < 1e-6);
        assert!((rule.frames[2].offset - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_parse_keyframes_unknown_property_skipped() {
        let sheet = CompiledStylesheet::parse(
            "@keyframes fade { 0% { not-a-real-prop: 42; opacity: 0; } 100% { opacity: 1; } }",
        );
        let rule = sheet.keyframes.get("fade").expect("fade registered");
        assert_eq!(rule.frames.len(), 2);
        // The unknown declaration is skipped; the opacity one is kept.
        let has_opacity =
            rule.frames[0].declarations.iter().any(|d| matches!(d, StyleDeclaration::Opacity(_)));
        assert!(has_opacity);
    }

    #[test]
    fn test_parse_animation_shorthand_full() {
        let decls = parse_decls(".x { animation: pulse-dot 2s ease-in-out infinite; }");
        let defs = decls
            .iter()
            .find_map(|d| match d {
                StyleDeclaration::Animation(v) => Some(v),
                _ => None,
            })
            .expect("animation decl");
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name.as_deref(), Some("pulse-dot"));
        assert!((defs[0].duration.as_secs_f32() - 2.0).abs() < 1e-3);
        assert_eq!(defs[0].timing_function, TimingFunction::EaseInOut);
        assert!(matches!(defs[0].iteration_count, types::IterationCount::Infinite));
    }

    #[test]
    fn test_parse_animation_shorthand_two_animations() {
        let decls = parse_decls(".x { animation: a 1s, b 2s ease-in 100ms; }");
        let defs = decls
            .iter()
            .find_map(|d| match d {
                StyleDeclaration::Animation(v) => Some(v),
                _ => None,
            })
            .expect("animation decl");
        assert_eq!(defs.len(), 2);
        assert_eq!(defs[0].name.as_deref(), Some("a"));
        assert!((defs[0].duration.as_secs_f32() - 1.0).abs() < 1e-3);
        assert_eq!(defs[1].name.as_deref(), Some("b"));
        assert!((defs[1].duration.as_secs_f32() - 2.0).abs() < 1e-3);
        assert_eq!(defs[1].timing_function, TimingFunction::EaseIn);
        assert!((defs[1].delay.as_secs_f32() - 0.1).abs() < 1e-3);
    }

    #[test]
    fn test_parse_animation_longhand_name() {
        let sheet = CompiledStylesheet::parse(".x { animation-name: foo; }");
        let rule = sheet.rules.iter().find(|r| !r.declarations.is_empty()).unwrap();
        let mut style = ComputedStyle::default();
        for d in &rule.declarations {
            apply_declaration(&mut style, d);
        }
        assert_eq!(style.animations.len(), 1);
        assert_eq!(style.animations[0].name.as_deref(), Some("foo"));
    }

    #[test]
    fn test_parse_animation_longhand_duration_and_delay() {
        let sheet = CompiledStylesheet::parse(
            ".x { animation-name: foo; animation-duration: 250ms; animation-delay: -100ms; }",
        );
        let rule = sheet.rules.iter().find(|r| !r.declarations.is_empty()).unwrap();
        let mut style = ComputedStyle::default();
        for d in &rule.declarations {
            apply_declaration(&mut style, d);
        }
        assert_eq!(style.animations.len(), 1);
        assert!((style.animations[0].duration.as_secs_f32() - 0.25).abs() < 1e-3);
        // Negative delay is preserved in the signed nanos field.
        assert_eq!(style.animations[0].delay_nanos, -100_000_000);
    }

    #[test]
    fn test_parse_animation_longhand_iteration_count() {
        let sheet = CompiledStylesheet::parse(
            ".x { animation-name: foo; animation-iteration-count: infinite; }",
        );
        let rule = sheet.rules.iter().find(|r| !r.declarations.is_empty()).unwrap();
        let mut style = ComputedStyle::default();
        for d in &rule.declarations {
            apply_declaration(&mut style, d);
        }
        assert!(matches!(style.animations[0].iteration_count, types::IterationCount::Infinite));
    }

    #[test]
    fn test_parse_animation_longhand_direction_fill_play_state() {
        let sheet = CompiledStylesheet::parse(
            ".x { \
                animation-name: foo; \
                animation-direction: alternate; \
                animation-fill-mode: forwards; \
                animation-play-state: paused; \
             }",
        );
        let rule = sheet.rules.iter().find(|r| !r.declarations.is_empty()).unwrap();
        let mut style = ComputedStyle::default();
        for d in &rule.declarations {
            apply_declaration(&mut style, d);
        }
        assert_eq!(style.animations[0].direction, types::AnimationDirection::Alternate);
        assert_eq!(style.animations[0].fill_mode, types::AnimationFillMode::Forwards);
        assert_eq!(style.animations[0].play_state, types::AnimationPlayState::Paused);
    }

    #[test]
    fn test_animation_name_none_clears() {
        let sheet =
            CompiledStylesheet::parse(".x { animation: pulse-dot 2s; animation-name: none; }");
        let rule = sheet.rules.iter().find(|r| !r.declarations.is_empty()).unwrap();
        let mut style = ComputedStyle::default();
        for d in &rule.declarations {
            apply_declaration(&mut style, d);
        }
        // After `animation-name: none`, the cascaded animation list has a
        // single entry with name=None, which the driver treats as cleared.
        assert_eq!(style.animations.len(), 1);
        assert!(style.animations[0].name.is_none());
    }

    #[test]
    fn test_terminal_manager_keyframes_parse() {
        let css = r#"
            @keyframes pulse-dot {
                0%, 100% { opacity: 1; }
                50% { opacity: 0.4; }
            }
            @keyframes cursor-blink {
                50% { opacity: 0; }
            }
            @keyframes fade-in {
                from { opacity: 0; }
                to { opacity: 1; }
            }
        "#;
        let sheet = CompiledStylesheet::parse(css);
        assert!(sheet.keyframes.contains_key("pulse-dot"));
        assert!(sheet.keyframes.contains_key("cursor-blink"));
        assert!(sheet.keyframes.contains_key("fade-in"));
        // cursor-blink only has a 50% entry; the driver synthesizes 0%/100%.
        assert_eq!(sheet.keyframes.get("cursor-blink").unwrap().frames.len(), 1);
    }

    // ------------------------------------------------------------------
    // backdrop-filter parsing (issue #134)
    // ------------------------------------------------------------------

    fn first_backdrop_filter(css: &str) -> Option<types::BackdropFilter> {
        let decls = parse_decls(css);
        decls.into_iter().find_map(|d| match d {
            StyleDeclaration::BackdropFilter(v) => Some(v),
            _ => None,
        })
    }

    #[test]
    fn test_backdrop_filter_blur_px() {
        let bf = first_backdrop_filter(".x { backdrop-filter: blur(6px); }")
            .expect("should parse blur(6px)");
        assert_eq!(bf.filters.len(), 1);
        match bf.filters[0] {
            types::FilterFunction::Blur(r) => assert!((r - 6.0).abs() < 0.001),
        }
    }

    #[test]
    fn test_backdrop_filter_blur_zero() {
        let bf = first_backdrop_filter(".x { backdrop-filter: blur(0); }")
            .expect("should parse blur(0)");
        assert_eq!(bf.filters.len(), 1);
        match bf.filters[0] {
            types::FilterFunction::Blur(r) => assert_eq!(r, 0.0),
        }
    }

    #[test]
    fn test_backdrop_filter_none() {
        // `none` must not produce a BackdropFilter declaration at all.
        let bf = first_backdrop_filter(".x { backdrop-filter: none; }");
        assert!(bf.is_none());
    }

    #[test]
    fn test_backdrop_filter_two_entry_list() {
        let bf = first_backdrop_filter(".x { backdrop-filter: blur(6px), blur(2px); }")
            .expect("should parse two entry list");
        assert_eq!(bf.filters.len(), 2);
        match bf.filters[0] {
            types::FilterFunction::Blur(r) => assert!((r - 6.0).abs() < 0.001),
        }
        match bf.filters[1] {
            types::FilterFunction::Blur(r) => assert!((r - 2.0).abs() < 0.001),
        }
    }

    #[test]
    fn test_backdrop_filter_drop_shadow_ignored() {
        // Non blur filter functions parse gracefully to no op, leaving the
        // filter list empty which suppresses the declaration entirely.
        let bf = first_backdrop_filter(".x { backdrop-filter: drop-shadow(0 0 2px red); }");
        assert!(bf.is_none());
    }

    #[test]
    fn test_backdrop_filter_missing_unit_parses_as_pixels() {
        let bf = first_backdrop_filter(".x { backdrop-filter: blur(6); }")
            .expect("should parse blur(6) as 6 pixels");
        assert_eq!(bf.filters.len(), 1);
        match bf.filters[0] {
            types::FilterFunction::Blur(r) => assert!((r - 6.0).abs() < 0.001),
        }
    }

    #[test]
    fn test_backdrop_filter_clamped_to_max_radius() {
        let bf = first_backdrop_filter(".x { backdrop-filter: blur(2000px); }")
            .expect("should clamp huge blur");
        assert_eq!(bf.filters.len(), 1);
        match bf.filters[0] {
            types::FilterFunction::Blur(r) => {
                assert_eq!(r, BACKDROP_FILTER_MAX_BLUR_RADIUS);
            }
        }
    }

    #[test]
    fn test_backdrop_filter_applied_to_computed_style() {
        use crate::style::types::ComputedStyle;

        let sheet = CompiledStylesheet::parse(".modal { backdrop-filter: blur(6px); }");
        assert_eq!(sheet.rules.len(), 1);
        let mut style = ComputedStyle::default();
        for decl in &sheet.rules[0].declarations {
            apply_declaration(&mut style, decl);
        }
        let bf = style.backdrop_filter.expect("backdrop_filter should be Some");
        assert_eq!(bf.filters.len(), 1);
        match bf.filters[0] {
            types::FilterFunction::Blur(r) => assert!((r - 6.0).abs() < 0.001),
        }
    }

    #[test]
    fn test_backdrop_filter_mixed_list_keeps_only_blur() {
        // A mixed filter list still produces a declaration when at least one
        // entry is recognized.
        let bf =
            first_backdrop_filter(".x { backdrop-filter: drop-shadow(0 0 2px red), blur(4px); }")
                .expect("should keep the blur entry");
        assert_eq!(bf.filters.len(), 1);
        match bf.filters[0] {
            types::FilterFunction::Blur(r) => assert!((r - 4.0).abs() < 0.001),
        }
    }

    /// Extract flex-grow, flex-shrink, flex-basis from a declaration list.
    fn extract_flex(decls: &[StyleDeclaration]) -> (Option<f32>, Option<f32>, Option<Dimension>) {
        let grow = decls.iter().find_map(|d| match d {
            StyleDeclaration::FlexGrow(v) => Some(*v),
            _ => None,
        });
        let shrink = decls.iter().find_map(|d| match d {
            StyleDeclaration::FlexShrink(v) => Some(*v),
            _ => None,
        });
        let basis = decls.iter().find_map(|d| match d {
            StyleDeclaration::FlexBasis(v) => Some(*v),
            _ => None,
        });
        (grow, shrink, basis)
    }

    #[test]
    fn test_flex_shorthand_single_number() {
        let (grow, shrink, basis) = extract_flex(&parse_decls(".x { flex: 1; }"));
        assert_eq!(grow, Some(1.0));
        assert_eq!(shrink, Some(1.0));
        assert_eq!(basis, Some(Dimension::Px(0.0)));
    }

    #[test]
    fn test_flex_shorthand_two_numbers() {
        let (grow, shrink, basis) = extract_flex(&parse_decls(".x { flex: 2 3; }"));
        assert_eq!(grow, Some(2.0));
        assert_eq!(shrink, Some(3.0));
        assert_eq!(basis, Some(Dimension::Px(0.0)));
    }

    #[test]
    fn test_flex_shorthand_three_values_auto() {
        let (grow, shrink, basis) = extract_flex(&parse_decls(".x { flex: 1 1 auto; }"));
        assert_eq!(grow, Some(1.0));
        assert_eq!(shrink, Some(1.0));
        assert_eq!(basis, Some(Dimension::Auto));
    }

    #[test]
    fn test_flex_shorthand_none() {
        let (grow, shrink, basis) = extract_flex(&parse_decls(".x { flex: none; }"));
        assert_eq!(grow, Some(0.0));
        assert_eq!(shrink, Some(0.0));
        assert_eq!(basis, Some(Dimension::Auto));
    }

    #[test]
    fn test_flex_shorthand_zero() {
        let (grow, shrink, basis) = extract_flex(&parse_decls(".x { flex: 0; }"));
        assert_eq!(grow, Some(0.0));
        assert_eq!(shrink, Some(1.0));
        assert_eq!(basis, Some(Dimension::Px(0.0)));
    }

    #[test]
    fn test_inset_shorthand_expands_to_all_four_edges() {
        let decls = parse_decls(".x { inset: 0; }");
        let has_top = decls.iter().any(|d| matches!(d, StyleDeclaration::Top(_)));
        let has_right = decls.iter().any(|d| matches!(d, StyleDeclaration::Right(_)));
        let has_bottom = decls.iter().any(|d| matches!(d, StyleDeclaration::Bottom(_)));
        let has_left = decls.iter().any(|d| matches!(d, StyleDeclaration::Left(_)));
        assert!(has_top, "inset should expand to top");
        assert!(has_right, "inset should expand to right");
        assert!(has_bottom, "inset should expand to bottom");
        assert!(has_left, "inset should expand to left");
    }

    #[test]
    fn test_inset_shorthand_px_value() {
        let decls = parse_decls(".x { inset: 10px; }");
        let top = decls.iter().find_map(|d| match d {
            StyleDeclaration::Top(v) => Some(*v),
            _ => None,
        });
        assert_eq!(top, Some(Dimension::Px(10.0)), "inset: 10px should expand top to 10px");
    }

    // Regression tests for #214: display: inline-flex and inline-block
    // were silently dropped because the parser returned Err(()) for
    // unrecognised display values.

    #[test]
    fn test_display_inline_flex() {
        let decls = parse_decls(".x { display: inline-flex; }");
        let display = decls.iter().find_map(|d| match d {
            StyleDeclaration::Display(v) => Some(*v),
            _ => None,
        });
        assert_eq!(display, Some(Display::InlineFlex));
    }

    #[test]
    fn test_display_inline_block() {
        let decls = parse_decls(".x { display: inline-block; }");
        let display = decls.iter().find_map(|d| match d {
            StyleDeclaration::Display(v) => Some(*v),
            _ => None,
        });
        assert_eq!(display, Some(Display::InlineBlock));
    }

    #[test]
    fn test_display_flex_still_works() {
        let decls = parse_decls(".x { display: flex; }");
        let display = decls.iter().find_map(|d| match d {
            StyleDeclaration::Display(v) => Some(*v),
            _ => None,
        });
        assert_eq!(display, Some(Display::Flex));
    }

    #[test]
    fn test_display_none_still_works() {
        let decls = parse_decls(".x { display: none; }");
        let display = decls.iter().find_map(|d| match d {
            StyleDeclaration::Display(v) => Some(*v),
            _ => None,
        });
        assert_eq!(display, Some(Display::None));
    }

    #[test]
    fn test_pseudo_element_placeholder_parses() {
        let parts = last_parts_of("input::placeholder { color: gray; }");
        assert!(parts
            .iter()
            .any(|p| matches!(p, SelectorPart::PseudoElement(PseudoElement::Placeholder))));
    }

    #[test]
    fn test_pseudo_element_placeholder_with_class() {
        let parts = last_parts_of(".field::placeholder { color: gray; }");
        let has_class = parts.iter().any(|p| matches!(p, SelectorPart::Class(c) if c == "field"));
        let has_placeholder = parts
            .iter()
            .any(|p| matches!(p, SelectorPart::PseudoElement(PseudoElement::Placeholder)));
        assert!(has_class && has_placeholder);
    }

    #[test]
    fn test_pseudo_element_webkit_input_placeholder_parses() {
        let parts = last_parts_of("input::-webkit-input-placeholder { color: gray; }");
        assert!(parts
            .iter()
            .any(|p| matches!(p, SelectorPart::PseudoElement(PseudoElement::Placeholder))));
    }

    #[test]
    fn test_pseudo_element_moz_placeholder_parses() {
        let parts = last_parts_of("input::-moz-placeholder { color: gray; }");
        assert!(parts
            .iter()
            .any(|p| matches!(p, SelectorPart::PseudoElement(PseudoElement::Placeholder))));
    }

    #[test]
    fn test_placeholder_alongside_before_after() {
        let sheet = CompiledStylesheet::parse(
            r#"
            .input::before { content: "*"; }
            .input::placeholder { color: gray; }
            .input::after { content: "!"; }
            "#,
        );
        assert_eq!(sheet.rules.len(), 3);
        let pseudo_elements: Vec<_> =
            sheet.rules.iter().filter_map(|r| r.selector.pseudo_element()).collect();
        assert!(pseudo_elements.contains(&PseudoElement::Before));
        assert!(pseudo_elements.contains(&PseudoElement::After));
        assert!(pseudo_elements.contains(&PseudoElement::Placeholder));
    }

    // -----------------------------------------------------------------
    // Cursor CSS property: new values
    // -----------------------------------------------------------------

    #[test]
    fn test_cursor_all_values() {
        let values = [
            ("default", CursorStyle::Default),
            ("auto", CursorStyle::Default),
            ("none", CursorStyle::None),
            ("pointer", CursorStyle::Pointer),
            ("text", CursorStyle::Text),
            ("grab", CursorStyle::Grab),
            ("grabbing", CursorStyle::Grabbing),
            ("not-allowed", CursorStyle::NotAllowed),
            ("crosshair", CursorStyle::Crosshair),
            ("move", CursorStyle::Move),
            ("wait", CursorStyle::Wait),
            ("help", CursorStyle::Help),
            ("progress", CursorStyle::Progress),
            ("col-resize", CursorStyle::ColResize),
            ("row-resize", CursorStyle::RowResize),
            ("n-resize", CursorStyle::NResize),
            ("s-resize", CursorStyle::SResize),
            ("e-resize", CursorStyle::EResize),
            ("w-resize", CursorStyle::WResize),
            ("ne-resize", CursorStyle::NeResize),
            ("nw-resize", CursorStyle::NwResize),
            ("se-resize", CursorStyle::SeResize),
            ("sw-resize", CursorStyle::SwResize),
            ("ew-resize", CursorStyle::EwResize),
            ("ns-resize", CursorStyle::NsResize),
            ("nesw-resize", CursorStyle::NeswResize),
            ("nwse-resize", CursorStyle::NwseResize),
            ("zoom-in", CursorStyle::ZoomIn),
            ("zoom-out", CursorStyle::ZoomOut),
        ];
        for (css_val, expected) in values {
            let css = format!(".x {{ cursor: {}; }}", css_val);
            let decls = parse_decls(&css);
            let cursor = decls.iter().find_map(|d| match d {
                StyleDeclaration::Cursor(v) => Some(*v),
                _ => None,
            });
            assert_eq!(cursor, Some(expected), "cursor: {css_val} should parse to {expected:?}");
        }
    }

    // -----------------------------------------------------------------
    // user-select CSS property
    // -----------------------------------------------------------------

    #[test]
    fn test_user_select_values() {
        let values = [
            ("auto", UserSelect::Auto),
            ("none", UserSelect::None),
            ("text", UserSelect::Text),
            ("all", UserSelect::All),
        ];
        for (css_val, expected) in values {
            let css = format!(".x {{ user-select: {}; }}", css_val);
            let decls = parse_decls(&css);
            let us = decls.iter().find_map(|d| match d {
                StyleDeclaration::UserSelect(v) => Some(*v),
                _ => None,
            });
            assert_eq!(us, Some(expected), "user-select: {css_val} should parse to {expected:?}");
        }
    }

    // -----------------------------------------------------------------
    // :focus-visible and :focus-within pseudo-class selectors
    // -----------------------------------------------------------------

    #[test]
    fn test_focus_visible_selector() {
        let parts = last_parts_of(".btn:focus-visible { outline: 2px solid blue; }");
        assert!(
            parts.iter().any(|p| matches!(p, SelectorPart::PseudoClass(PseudoClass::FocusVisible))),
            "should parse :focus-visible pseudo-class"
        );
    }

    #[test]
    fn test_focus_within_selector() {
        let parts = last_parts_of(".container:focus-within { border-color: #00ff00; }");
        assert!(
            parts.iter().any(|p| matches!(p, SelectorPart::PseudoClass(PseudoClass::FocusWithin))),
            "should parse :focus-within pseudo-class"
        );
    }
}
