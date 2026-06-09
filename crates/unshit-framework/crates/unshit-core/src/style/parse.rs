use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::cursor::CursorShape;
use crate::style::transition::{TimingFunction, TransitionDef, TransitionProperty};
use crate::style::types;
use crate::style::types::*;
use cssparser::{Parser, ParserInput, Token};
use smallvec::SmallVec;

/// A declaration the parser could not turn into a typed `StyleDeclaration` and
/// therefore silently discarded — either an unrecognized property or a value
/// the property's parser rejected (e.g. a viewport unit on a px-only pathway,
/// or `calc()`). Collected so callers can surface engine gaps instead of
/// discovering them when something renders wrong. Custom-property definitions
/// (`--name: ...`) are recorded too; consumers that only care about real
/// property gaps should filter those out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DroppedDeclaration {
    /// Raw selector text of the rule the declaration appeared in.
    pub selector: String,
    /// Property name (text before the first `:`).
    pub property: String,
    /// Raw value text (after the first `:`, sans trailing `;`).
    pub value: String,
}

impl DroppedDeclaration {
    /// True for custom-property definitions (`--name: ...`), which are not a
    /// per-property engine gap (they are handled by parse-time var resolution).
    pub fn is_custom_property(&self) -> bool {
        self.property.starts_with("--")
    }
}

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
    /// Declarations the parser discarded (see `DroppedDeclaration`). Empty for
    /// a fully-supported stylesheet; used by the dev-mode warning and the
    /// `stylesheet_coverage` guardrail test to surface engine gaps.
    pub dropped: Vec<DroppedDeclaration>,
    /// Cascade-aware custom-property collection. Every block that declares at
    /// least one `--name:` becomes a [`TokenScope`] here (with `:root`/`*`
    /// collapsed into the base scope 0). Consumed by the cascade (Stage 3): a
    /// `Deferred` declaration resolves its `var()` against the matching scopes,
    /// so themed `--token` overrides win per element.
    pub token_scopes: TokenScopes,
    /// Process-unique id assigned at [`Self::parse`] time. Used as the
    /// invalidation key for the cascade's deferred-resolution memo so a re-parse
    /// (hot reload, or just a different stylesheet) can never serve a stale memo
    /// entry — and, unlike a heap address, it is immune to allocator ABA reuse.
    /// `0` for a `Default` (never-parsed) stylesheet, which the memo treats as
    /// "uncached".
    pub parse_id: u64,
}

/// Interned handle for a token scope's selector text. The value is the scope's
/// stable index into [`TokenScopes::scopes`], assigned in source order at
/// collection time. Two scopes that share the exact trimmed selector text
/// share one key (the first occurrence wins; later declarations merge into it),
/// so callers can compare scopes cheaply by key instead of re-parsing
/// selector strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScopeKey(pub u32);

/// One CSS block that declared at least one custom property (`--name: value`).
///
/// The base scope (key 0) is the collapse of every `:root` and `*` block. Every
/// other selector that carries `--token` declarations (each `.app.theme-*`
/// block, each `.theme-chip.<name>` block, etc.) gets its own scope.
///
/// `vars` holds the scope's tokens with their values stored RAW (verbatim CSS
/// text), including any `var()` cross-token references. References are NOT
/// pre-flattened: a base-scope alias like `--cp-accent: var(--amber-300)` stays
/// raw so a theme's `--amber-300` override propagates to every consumer that
/// reaches the alias. Token->token references are unwound LAZILY and
/// MULTI-LEVEL at use time by `flatten_token_value_env`, looking each name up in
/// the element's full `ScopeEnv` (highest-specificity scope first) every pass.
#[derive(Debug, Clone)]
pub struct TokenScope {
    /// Stable interned handle for this scope (its index in `scopes`).
    pub key: ScopeKey,
    /// Trimmed raw selector text of the block, e.g. `".app.theme-dracula"`.
    /// The base scope uses the literal `":root"`.
    pub selector_text: String,
    /// CSS specificity of `selector_text` (ids, classes, tags). The base scope
    /// records `:root`'s specificity. Used by later stages to order overlapping
    /// scope overrides; unused this stage.
    pub specificity: (u16, u16, u16),
    /// Source order of the FIRST block that contributed to this scope, counting
    /// every brace-delimited block (custom-property-bearing or not) from the top
    /// of the stylesheet. Ties in specificity break on this, matching the
    /// cascade's specificity+source-order ordering.
    pub source_order: u32,
    /// Parsed selector chain for `selector_text`, used by the cascade to match
    /// the scope against an element via the SAME `selector_matches` path the
    /// rule cascade uses (so a theme scope `.app.theme-dracula` matches a root
    /// carrying both classes, and a widget scope `.theme-chip.dracula` matches
    /// the element itself). `None` for the base `:root`/`*` scope (never matched
    /// positionally — it is always active) and for any selector that fails to
    /// parse.
    pub selector: Option<SelectorChain>,
    /// Pre-flattened `--name -> resolved value` map for this scope (see the
    /// type doc). Shared via `Arc` so later stages can clone the handle cheaply.
    pub vars: Arc<HashMap<String, String>>,
}

/// Cascade-aware custom-property scopes collected from the stylesheet. Scope 0
/// is the base (`:root`/`*` collapsed); the rest follow in source order.
#[derive(Debug, Clone, Default)]
pub struct TokenScopes {
    pub scopes: Vec<TokenScope>,
    /// Union of every class name that appears in the TERMINAL compound (the part
    /// matched against the element itself) of a NON-base (widget) scope's
    /// selector. Precomputed once at collection time so the cascade can cheaply
    /// gate the per-element self-scope walk: an element whose classes do not
    /// intersect this set cannot match any widget scope's terminal compound, so
    /// the `O(non_base scopes)` `selector_matches` walk is skipped entirely (the
    /// overwhelmingly common case). Empty when there are no non-base scopes.
    pub widget_scope_classes: std::collections::HashSet<String>,
    /// True if at least one non-base scope's selector has a TERMINAL compound
    /// with NO class part (e.g. an id-only or tag-only terminal). Such a scope
    /// could match an element that shares none of [`Self::widget_scope_classes`],
    /// so the class-intersection gate is UNSAFE and must be disabled (the
    /// self-scope walk runs for every element). False for the common all-class
    /// stylesheet, keeping the gate live.
    pub widget_scope_gate_unsafe: bool,
}

impl TokenScopes {
    /// The base scope (`:root`/`*`), if any block declared custom properties.
    pub fn base(&self) -> Option<&TokenScope> {
        self.scopes.first()
    }

    /// True if `element_classes` could match some widget (non-base) self scope's
    /// terminal compound, so the cascade must run the self-scope walk. When
    /// false, the element cannot match any widget self scope and the walk is
    /// skipped. Conservatively returns true (never skips) when a non-base scope
    /// has a class-free terminal compound (the gate is then unsafe). Always
    /// false when there are no non-base scopes.
    pub fn element_may_have_self_scope(&self, element_classes: &[String]) -> bool {
        if self.widget_scope_classes.is_empty() && !self.widget_scope_gate_unsafe {
            return false;
        }
        if self.widget_scope_gate_unsafe {
            return true;
        }
        element_classes.iter().any(|c| self.widget_scope_classes.contains(c))
    }

    /// Look up a scope by its exact trimmed selector text.
    pub fn by_selector(&self, selector: &str) -> Option<&TokenScope> {
        self.scopes.iter().find(|s| s.selector_text == selector)
    }

    /// The base (`:root`/`*`) pre-flattened var map, or `None` when the
    /// stylesheet declared no base custom properties.
    pub fn base_vars(&self) -> Option<&HashMap<String, String>> {
        self.base().map(|s| s.vars.as_ref())
    }

    /// The non-base scopes (every `.app.theme-*`, widget scope, etc.) in source
    /// order. The cascade matches these against an element's classes to pick the
    /// active root theme scope and any widget self scope.
    pub fn non_base(&self) -> &[TokenScope] {
        // Scope 0 is the base when present; if there is no base scope at all the
        // first scope is already a non-base one, so guard on `base()`.
        if self.base().is_some() {
            self.scopes.get(1..).unwrap_or(&[])
        } else {
            &self.scopes
        }
    }

    /// The var map for a scope key, if the key is in range.
    pub fn vars_for(&self, key: ScopeKey) -> Option<&HashMap<String, String>> {
        self.scopes.get(key.0 as usize).map(|s| s.vars.as_ref())
    }
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
    FirstOfType,
    LastOfType,
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

/// A parsed `text-shadow` layer before color resolution. `color` is `None`
/// when omitted; the resolver fills it from the element's `color` at apply
/// time (CSS `currentColor` default), exactly like [`ParsedBoxShadow`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParsedTextShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub color: Option<Color>,
}

/// Identifies one edge of a box for per-side longhand CSS properties
/// (e.g. `border-top-width`). Kept separate from the geometric `Edges`
/// struct so the parser can carry the side tag in a declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderSide {
    Top,
    Right,
    Bottom,
    Left,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StyleDeclaration {
    /// A declaration whose value still contains `var(` at the moment
    /// [`parse_declaration`] sees it, captured verbatim instead of being typed
    /// eagerly. The concrete value is resolved PER ELEMENT in the cascade against
    /// that element's active token scopes, and only then re-parsed into a real
    /// typed declaration (see [`apply_deferred_against_env`]).
    ///
    /// `raw_value` is the source text after the `:` (sans trailing `;`), kept as
    /// a `Box<str>` rather than a `Vec<Token>` so the variant dodges the
    /// cssparser token-lifetime hazard and keeps [`StyleDeclaration`] cheap to
    /// `Clone` (the declaration vec is cloned once per grouped selector).
    ///
    /// `scope_hint` is the [`ScopeKey`] of the block this declaration was written
    /// in, threaded through [`parse_rule`]; it is a diagnostic/backstop label for
    /// the resolve, not the primary source — the element's [`ScopeEnv`] encodes
    /// its active scopes in specificity order.
    ///
    /// Stage 3 deleted the global parse-time `var()` substitution, so this is now
    /// the LIVE carrier for every `var(`-bearing declaration in a production
    /// parse (declarations with no `var(` keep the typed fast path).
    Deferred {
        property: Box<str>,
        raw_value: Box<str>,
        scope_hint: ScopeKey,
    },
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
    /// Padding authored with viewport/percent units (`vh`/`vw`/`%`), kept
    /// unresolved per side (`[top, right, bottom, left]`, `None` = leave that
    /// side untouched) so `to_taffy_style` can resolve it against the viewport.
    /// Pure-`px` padding keeps the f32 fast path above.
    PaddingDim([Option<Dimension>; 4]),
    Margin(Edges),
    MarginWithAuto(Edges, EdgeAutoFlags),
    MarginTop(f32),
    MarginRight(f32),
    MarginBottom(f32),
    MarginLeft(f32),
    MarginTopAuto,
    MarginRightAuto,
    MarginBottomAuto,
    MarginLeftAuto,
    Gap(f32),
    RowGap(f32),
    ColumnGap(f32),
    OverflowX(Overflow),
    OverflowY(Overflow),
    Background(types::Background),
    BorderColor(Color),
    BorderWidth(Edges),
    /// Per-side `border-<side>-width` longhand. CSS lets an author set
    /// just one or two sides (`border-left-width`, `border-top-width`)
    /// which is lossy through the shorthand `border-width` value, so
    /// each side has its own declaration slot.
    BorderSideWidth(BorderSide, f32),
    /// Per-side `border-<side>-color` longhand. The engine stores a single
    /// `border_color`, so every side writes that one field; this is visually
    /// exact whenever only one side has a non-zero width (the case for every
    /// authored consumer). Differently-colored adjacent sides would collapse
    /// to last-writer-wins, which no current stylesheet relies on.
    BorderSideColor(BorderSide, Color),
    BorderRadius(Corners),
    /// `border-radius` authored with percent corners (`50%` circular avatars),
    /// kept unresolved per corner so the renderer can resolve them against the
    /// element box at paint time. Pure-`px` radii keep the `BorderRadius` f32
    /// fast path above (preserving transition / scale behavior).
    BorderRadiusDim(CornersDim),
    Opacity(f32),
    BoxShadowList(SmallVec<[ParsedBoxShadow; 2]>),
    TextShadowList(SmallVec<[ParsedTextShadow; 2]>),
    BackdropFilter(types::BackdropFilter),
    Color(Color),
    FontSize(f32),
    /// Runtime text scale applied after the CSS cascade.
    ///
    /// This is intentionally not parsed from CSS today; app builders use it as
    /// an inline override when user settings need to scale a subtree while
    /// preserving the stylesheet's relative text hierarchy.
    FontScale(f32),
    FontWeight(FontWeight),
    FontStyle(FontStyle),
    FontFamily(String),
    LineHeight(f32),
    LetterSpacing(f32),
    TextAlign(TextAlign),
    TextTransform(TextTransform),
    TextDecoration(TextDecoration),
    TextDecorationColor(Color),
    WhiteSpace(types::WhiteSpace),
    TextOverflow(types::TextOverflow),
    Cursor(CursorStyle),
    Visibility(Visibility),
    PointerEvents(PointerEvents),
    AppRegion(AppRegion),
    Position(CssPosition),
    Top(Dimension),
    Right(Dimension),
    Bottom(Dimension),
    Left(Dimension),
    ZIndex(i32),
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

    // CSS resize
    Resize(types::CssResize),

    // Box model
    BoxSizing(types::BoxSizing),
    AspectRatio(Option<f32>),
    ObjectFit(types::ObjectFit),
    ObjectPosition(types::ObjectPosition),

    // Resize handle
    ResizeAxis(crate::resize_handle::ResizeAxis),

    // Bell / notification
    BellStyle(types::BellStyle),

    /// `transform: <function-list>` (translate / scale / rotate). See
    /// `parse_transform` for the accepted forms; composed into an affine by
    /// the renderer.
    Transform(types::Transform),

    /// `mask-image: linear-gradient(...)`. Any non gradient mask source
    /// (url, image(), none) parses to an error today. The linear gradient
    /// branch is reused verbatim from `parse_linear_gradient` so the stop
    /// list and fixup pass behave identically to a background gradient.
    MaskImage(types::LinearGradient),
}

impl CompiledStylesheet {
    pub fn parse(css: &str) -> Self {
        let custom_properties = extract_custom_properties(css);
        // Cascade-aware scope collection. Drives per-element var() resolution in
        // the cascade (Stage 3): every `--token` block — `:root` AND the
        // `.app.theme-*` / widget scopes — is collected here so a themed
        // override can win at apply time.
        let token_scopes = collect_token_scopes(css);

        // The global parse-time `resolve_var_references` textual pass is GONE.
        // `var(`-bearing declarations now reach `parse_declaration` with `var(`
        // intact and become `StyleDeclaration::Deferred` carriers, resolved
        // per element against its active token scopes during the cascade (see
        // `resolve_deferred_against_env`). Declarations with no `var(` keep the
        // byte-for-byte typed fast path. `:root`-only consumers that still need
        // the flat map read `custom_properties` above.
        let mut input = ParserInput::new(css);
        let mut parser = Parser::new(&mut input);
        let mut rules = Vec::new();
        let mut font_faces = Vec::new();
        let mut keyframes: HashMap<String, KeyframesRule> = HashMap::new();
        let mut dropped: Vec<DroppedDeclaration> = Vec::new();
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
            // extra token skip is needed on the error path. A grouped
            // selector like `.a, .b { ... }` returns one rule per sub
            // selector; each gets its own source_order so cascade order
            // matches the declaration order browsers use.
            if let Ok(mut new_rules) =
                parse_rule(&mut parser, source_order, &mut dropped, &token_scopes)
            {
                source_order += new_rules.len() as u32;
                rules.append(&mut new_rules);
            }
        }

        rules.sort_by(|a, b| {
            a.specificity.cmp(&b.specificity).then(a.source_order.cmp(&b.source_order))
        });

        // Coverage pass for cascade-time `var()` resolution failures. The live
        // per-element cascade applies `Deferred` carriers against an element's
        // `ScopeEnv` and routes any failure to a per-element sink that is then
        // discarded (it has no shared `dropped` list — see
        // `resolve_style_with_pseudo`). So a malformed/cyclic scoped `var()`
        // would silently mis-render with no signal to the
        // `stylesheet_coverage` guardrail. This dry-run resolves every `Deferred`
        // carrier against the base scope AND each collected theme/widget scope and
        // records any value that cannot resolve (an unresolved `var()` with no
        // token and no fallback, or a re-parse failure) into `dropped`, so the
        // gap is visible at parse time without slowing the per-element path.
        record_deferred_coverage_drops(&rules, &token_scopes, &mut dropped);

        // Process-unique id (starts at 1 so a `Default` stylesheet's `0` is
        // distinguishable) for the cascade's deferred-resolution memo invalidation.
        static PARSE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let parse_id = PARSE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        CompiledStylesheet {
            rules,
            custom_properties,
            font_faces,
            keyframes,
            dropped,
            token_scopes,
            parse_id,
        }
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
/// Remove `/* ... */` comments from CSS text. Used before the naive
/// brace/`;`-splitting in `extract_custom_properties`, which would otherwise
/// glue a comment to the following declaration (a comment containing `:` or
/// sitting before a `--custom` property silently breaks that property's
/// collection). The full rule parser uses cssparser, which strips comments
/// itself, so this is only needed for the custom-property pre-scan.
fn strip_css_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find("*/") {
            Some(end) => rest = &rest[start + 2 + end + 2..],
            // Unterminated comment: drop the remainder, matching CSS tokenizing.
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

fn extract_custom_properties(css: &str) -> HashMap<String, String> {
    let css = strip_css_comments(css);
    let css = css.as_str();
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

/// Parse the `--name: value;` declarations out of a block's inner text (the
/// span between its braces). Returns the raw, unresolved values exactly as
/// authored, keyed by full property name (`--name`). Mirrors the naive
/// `;`/`:`-split that `extract_custom_properties` uses on `:root`, so the two
/// agree on what counts as a custom-property declaration. Non-custom
/// declarations (e.g. `background: ...`) are ignored.
fn parse_block_custom_props(block: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for decl in block.split(';') {
        let decl = decl.trim();
        if decl.is_empty() {
            continue;
        }
        if let Some(colon_pos) = decl.find(':') {
            let name = decl[..colon_pos].trim();
            let value = decl[colon_pos + 1..].trim();
            if name.starts_with("--") {
                out.insert(name.to_string(), value.to_string());
            }
        }
    }
    out
}

/// Resolve `var(--name)` references inside a single token value against `props`,
/// iterating so a value that resolves to another `var()` keeps unwinding. Reuses
/// [`resolve_var_once`] (the same single-pass substitution the global resolver
/// uses) and bounds the work with a `visited` cycle guard: the moment a value
/// stops changing, or a name reappears on the resolution path, we stop. Used by
/// the per-scope pre-flatten; does not touch the global resolve.
fn flatten_token_value(raw: &str, props: &HashMap<String, String>) -> String {
    let mut value = raw.to_string();
    // Cap iterations as a hard backstop (mirrors `resolve_var_references`'s
    // fixed bound); the `visited` set is the real cycle guard below.
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    for _ in 0..32 {
        if !value.contains("var(") {
            break;
        }
        // If this exact text has been seen before, a cycle (or a fixed point we
        // cannot make progress on) is in play: stop and keep what we have.
        if !visited.insert(value.clone()) {
            break;
        }
        match resolve_var_once(&value, props) {
            Some(next) => value = next,
            None => break,
        }
    }
    value
}

/// Cascade-aware custom-property collection.
///
/// Walks EVERY brace-delimited block in source order using the same naive
/// brace-walker as [`extract_custom_properties`] (`strip_css_comments` +
/// `find_matching_brace`). For each block whose inner text declares at least one
/// `--name:` property, the block's trimmed selector text is interned as a
/// [`ScopeKey`] and that block's `--name -> raw value` map is recorded.
///
/// `:root` and `*` blocks collapse into scope 0 (the base): their declarations
/// merge (later blocks override earlier ones, matching the cascade). Every other
/// custom-property-bearing selector gets its own scope.
///
/// Token values are stored RAW (not pre-flattened): a `var()` cross-token
/// reference such as `--cp-accent: var(--amber-300)` is kept verbatim so a theme
/// that overrides `--amber-300` is seen by every consumer reaching it through
/// the alias. The reference is unwound lazily and multi-level at use time
/// against the element's `ScopeEnv` (see `flatten_token_value_env`).
fn collect_token_scopes(css: &str) -> TokenScopes {
    let css = strip_css_comments(css);
    let css = css.as_str();

    // Raw (unflattened) per-scope maps, in collection order. Index 0 is reserved
    // for the base (:root/*) scope; it is created lazily on first encounter.
    struct RawScope {
        selector_text: String,
        specificity: (u16, u16, u16),
        source_order: u32,
        vars: HashMap<String, String>,
    }
    let mut raw_scopes: Vec<RawScope> = Vec::new();
    // Selector text -> index into `raw_scopes`, so repeat selectors merge.
    let mut by_selector: HashMap<String, usize> = HashMap::new();
    // The base scope (`:root`/`*`) shares one slot regardless of which literal
    // selector introduced it; this is its index once created.
    let mut base_index: Option<usize> = None;

    // `source_order` counts EVERY block (custom-property-bearing or not), so it
    // lines up with the cascade's block ordering even when some blocks declare
    // no tokens.
    let mut block_order: u32 = 0;
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

        let this_order = block_order;
        block_order += 1;

        let block = &css[brace_open + 1..brace_close];
        let props = parse_block_custom_props(block);
        if !props.is_empty() {
            let is_base = selector == ":root" || selector == "*";
            if is_base {
                let idx = match base_index {
                    Some(idx) => idx,
                    None => {
                        let idx = raw_scopes.len();
                        raw_scopes.push(RawScope {
                            // Canonicalize the base scope's label to `:root`
                            // even if `*` introduced it first.
                            selector_text: ":root".to_string(),
                            specificity: selector_specificity(":root"),
                            source_order: this_order,
                            vars: HashMap::new(),
                        });
                        base_index = Some(idx);
                        idx
                    }
                };
                // Later base declarations override earlier ones.
                raw_scopes[idx].vars.extend(props);
            } else {
                let key = selector.to_string();
                match by_selector.get(&key) {
                    Some(&idx) => {
                        // Same selector seen again: merge, later wins.
                        raw_scopes[idx].vars.extend(props);
                    }
                    None => {
                        let idx = raw_scopes.len();
                        raw_scopes.push(RawScope {
                            selector_text: key.clone(),
                            specificity: selector_specificity(selector),
                            source_order: this_order,
                            vars: props,
                        });
                        by_selector.insert(key, idx);
                    }
                }
            }
        }

        search_start = brace_close + 1;
    }

    // Ensure the base scope is index 0 so callers can rely on `scopes[0]` /
    // `TokenScopes::base()` being the base. If a non-base scope was collected
    // before any `:root`/`*` block (unusual but legal), move the base to front.
    if let Some(base_idx) = base_index {
        if base_idx != 0 {
            let base = raw_scopes.remove(base_idx);
            raw_scopes.insert(0, base);
        }
    }

    // Class names in the TERMINAL compound of each NON-base scope selector, for
    // the cascade's self-scope perf gate (see `TokenScopes::widget_scope_classes`).
    // `gate_unsafe` is set if any non-base scope has a class-free terminal, which
    // makes the class-intersection gate unsound (it must then not skip).
    let mut widget_scope_classes: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut widget_scope_gate_unsafe = false;

    let scopes: Vec<TokenScope> = raw_scopes
        .into_iter()
        .enumerate()
        .map(|(i, raw)| {
            let is_base = i == 0 && base_index.is_some();
            // Store RAW token values verbatim — do NOT eagerly concretize a
            // cross-token `var()` reference here. A base-scope alias such as
            // `--cp-accent: var(--amber-300)` must stay raw so a theme that
            // overrides `--amber-300` is seen by every consumer that reaches it
            // through the alias: the value is resolved LAZILY and MULTI-LEVEL at
            // use time against the element's full `ScopeEnv` (highest-specificity
            // scope first, then `:root`, then the var() fallback) by
            // `flatten_token_value_env`, which keeps unwinding token->token refs
            // against the same env each pass. Pre-flattening here would bind the
            // alias to `:root`'s `--amber-300` and silently break theme overrides.
            let vars: HashMap<String, String> = raw.vars.clone();
            // The base scope is always active and never matched positionally, so
            // it carries no parsed selector. Every other scope parses its
            // selector once here so the cascade can reuse `selector_matches`.
            let selector =
                if is_base { None } else { parse_selector_string(&raw.selector_text).ok() };
            // Feed the self-scope perf gate from each non-base scope selector's
            // TERMINAL compound (the part matched against the element itself). A
            // self-scope match requires the element to carry that compound's
            // classes, so an element sharing none of them cannot match. A scope
            // whose terminal carries no class (id-only/tag-only) makes the gate
            // unsafe. The active root theme scope is matched on the root, not the
            // element, so over-including its classes here only widens the gate
            // (harmless); the base `:root`/`*` is never a widget scope.
            if !is_base {
                if let Some(chain) = selector.as_ref() {
                    if !collect_terminal_classes(chain, &mut widget_scope_classes) {
                        widget_scope_gate_unsafe = true;
                    }
                } else {
                    // Selector failed to parse: cannot reason about its terminal,
                    // so disable the gate to stay correct.
                    widget_scope_gate_unsafe = true;
                }
            }
            TokenScope {
                key: ScopeKey(i as u32),
                selector_text: raw.selector_text,
                specificity: raw.specificity,
                source_order: raw.source_order,
                selector,
                vars: Arc::new(vars),
            }
        })
        .collect();

    TokenScopes { scopes, widget_scope_classes, widget_scope_gate_unsafe }
}

/// Collect the `Class(name)` parts of `chain`'s TERMINAL compound (the last
/// compound selector — the part `selector_matches` tests against the element
/// itself) into `out`. Returns `true` if that terminal compound has at least one
/// class part; `false` if it is class-free (id-only / tag-only / universal), in
/// which case the class-intersection self-scope gate cannot be applied safely.
/// `:not(.cls)` in the terminal counts as a positive class constraint only when
/// it is the sole signal — to stay conservative it is NOT treated as a required
/// class (a `:not` does not require the element to CARRY the class), so a
/// terminal whose only class-like part is a `:not` is reported as class-free.
fn collect_terminal_classes(
    chain: &SelectorChain,
    out: &mut std::collections::HashSet<String>,
) -> bool {
    let Some((parts, _)) = chain.parts.last() else {
        return false;
    };
    let mut found = false;
    for part in parts {
        if let SelectorPart::Class(name) = part {
            out.insert(name.clone());
            found = true;
        }
    }
    found
}

/// Specificity of a raw selector string, for [`TokenScope::specificity`]. Parses
/// the selector with the same path the rule parser uses and reuses
/// [`compute_specificity`]; an unparseable selector falls back to `(0, 0, 0)`.
fn selector_specificity(selector: &str) -> (u16, u16, u16) {
    parse_selector_string(selector).map(|chain| compute_specificity(&chain)).unwrap_or((0, 0, 0))
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

/// True if `value` contains a real `var(` FUNCTION call (not a substring of a
/// longer identifier like `myvar(` or a path like `url(.../myvar(1).png)`).
///
/// CSS function names are `ident(`, so a `var(` is a function only when the byte
/// immediately before `var` is a value boundary: the start of the value,
/// whitespace, an opening `(` (nested inside another function), or a `,`
/// (a comma-separated list item). Any other preceding byte (a letter, digit, or
/// `-`/`_`) means `var` is the tail of a longer identifier, so it is NOT a
/// `var()` call. The check stays a cheap byte scan over the typical no-`var(`
/// fast path: it only does the boundary test at each `var(` occurrence.
fn contains_var_function(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut search_from = 0;
    while let Some(rel) = value[search_from..].find("var(") {
        let pos = search_from + rel;
        let boundary = match pos.checked_sub(1).map(|i| bytes[i]) {
            None => true, // `var(` at the very start of the value
            Some(b) => {
                b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' || b == b'(' || b == b','
            }
        };
        if boundary {
            return true;
        }
        search_from = pos + 4; // past this (non-function) "var(" occurrence
    }
    false
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

fn parse_rule(
    parser: &mut Parser,
    source_order: u32,
    dropped: &mut Vec<DroppedDeclaration>,
    token_scopes: &TokenScopes,
) -> Result<Vec<CompiledRule>, ()> {
    let selector_str = collect_selector_text(parser)?;
    // ScopeKey of the block these declarations lexically live in. A grouped
    // selector (`.a, .b { ... }`) shares one block, so the hint is resolved from
    // the full, trimmed selector text. Selectors without their own
    // custom-property scope (the common case) fall back to the base scope (key
    // 0). Threaded into `parse_declaration` so any `Deferred` it captures knows
    // which scope's token overrides to layer over `:root` later.
    let scope_hint =
        token_scopes.by_selector(selector_str.trim()).map(|s| s.key).unwrap_or(ScopeKey(0));
    // Split comma-separated selector groups (`.a, .b, .c`) into individual
    // selectors. Each becomes its own compiled rule with a shared copy of
    // the declarations, matching CSS grouped selector semantics.
    let selector_parts: Vec<&str> = split_top_level_commas(&selector_str);
    let mut selectors: Vec<(SelectorChain, (u16, u16, u16))> =
        Vec::with_capacity(selector_parts.len());
    for part in selector_parts {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        match parse_selector_string(trimmed) {
            Ok(s) => {
                let spec = compute_specificity(&s);
                selectors.push((s, spec));
            }
            // A single bad branch in a group should not poison the rest,
            // but the parser already consumed the selector slice before the
            // block so we fall through to drain and return Err only if we
            // end up with no valid selectors at all.
            Err(()) => continue,
        }
    }
    if selectors.is_empty() {
        drain_nested_block(parser);
        return Err(());
    }

    let selector_for_diag = selector_str.as_str();
    let declarations = parser
        .parse_nested_block(|parser| {
            let mut decls = Vec::new();
            while !parser.is_exhausted() {
                let start = parser.position();
                if let Ok(parsed) = parse_declaration(parser, scope_hint) {
                    decls.extend(parsed);
                } else {
                    // The declaration could not be typed. Drain to the next
                    // semicolon (unchanged behavior) and record the raw text so
                    // callers can see which CSS the engine silently discarded.
                    while let Ok(token) = parser.next() {
                        if matches!(token, Token::Semicolon) {
                            break;
                        }
                    }
                    record_dropped_declaration(
                        dropped,
                        selector_for_diag,
                        parser.slice_from(start),
                    );
                }
            }
            Ok(decls)
        })
        .map_err(|_: cssparser::ParseError<'_, ()>| ())?;

    Ok(selectors
        .into_iter()
        .enumerate()
        .map(|(i, (selector, specificity))| CompiledRule {
            selector,
            specificity,
            declarations: declarations.clone(),
            source_order: source_order + i as u32,
        })
        .collect())
}

/// Record a declaration the parser failed to type. `raw` is the source slice
/// from the start of the declaration up to (and possibly including) its
/// terminating semicolon.
fn record_dropped_declaration(out: &mut Vec<DroppedDeclaration>, selector: &str, raw: &str) {
    let raw = raw.trim().trim_end_matches(';').trim();
    if raw.is_empty() {
        return;
    }
    let (property, value) = match raw.split_once(':') {
        Some((p, v)) => (p.trim(), v.trim()),
        None => (raw, ""),
    };
    // Only record real authored declarations: a CSS property name is an
    // identifier (`letters/digits/-`, optionally `--`-prefixed). This drops the
    // fragments a value parser can leave behind when it consumes only part of a
    // multi-value declaration (e.g. the trailing layer of a multi-layer
    // `background`), which are parser-state artifacts, not authored properties.
    if !is_css_property_name(property) {
        return;
    }
    out.push(DroppedDeclaration {
        selector: selector.trim().to_string(),
        property: property.to_string(),
        value: value.to_string(),
    });
}

fn is_css_property_name(s: &str) -> bool {
    match s.chars().next() {
        Some(c) if c.is_ascii_alphabetic() || c == '-' => {}
        _ => return false,
    }
    s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Parse-time coverage pass for cascade-time `var()` resolution failures.
///
/// The live cascade resolves each [`StyleDeclaration::Deferred`] carrier against
/// the matched element's [`ScopeEnv`] and routes any failure into a per-element
/// sink that is discarded — so a malformed/cyclic scoped `var()` mis-renders with
/// no signal the `stylesheet_coverage` guardrail can observe. This dry-run
/// resolves every `Deferred` carrier against the base scope AND each collected
/// non-base scope (used as the active root), and records any carrier that FAILS
/// to resolve+re-parse under EVERY tested env into `dropped`, so the gap is
/// visible at parse time. Recording only carriers that fail under every env keeps
/// a value that any real theme resolves (the common case) from being flagged.
///
/// De-duped on `(property, raw_value)`: a carrier that fails for many scopes is
/// recorded once. The recorded `value` is the resolved (concrete-as-far-as-
/// possible) text, so the guardrail's known-gap classifier can see it.
fn record_deferred_coverage_drops(
    rules: &[CompiledRule],
    token_scopes: &TokenScopes,
    dropped: &mut Vec<DroppedDeclaration>,
) {
    let base = token_scopes.base_vars();
    let non_base = token_scopes.non_base();
    // A stylesheet with no token scopes at all still has `:root`-less carriers
    // (e.g. `var()` with a fallback), so always include the base (possibly None)
    // env. Track which carriers we have already recorded.
    let mut seen: std::collections::HashSet<(Box<str>, Box<str>)> =
        std::collections::HashSet::new();

    for rule in rules {
        for decl in &rule.declarations {
            let StyleDeclaration::Deferred { property, raw_value, .. } = decl else {
                continue;
            };
            let key = (Box::<str>::from(property.as_ref()), Box::<str>::from(raw_value.as_ref()));
            if seen.contains(&key) {
                continue;
            }

            // Resolve against the base-only env and against each non-base scope
            // as the active root. The carrier is a coverage failure only if it
            // fails under EVERY tested env (a value some theme resolves is fine).
            let mut failure: Option<String> = None;
            let mut any_ok = false;

            let base_env = ScopeEnv::new(None, None, base);
            match resolve_deferred_to_decls(property, raw_value, &base_env) {
                Ok(_) => any_ok = true,
                Err(resolved) => failure = Some(resolved),
            }

            if !any_ok {
                for scope in non_base {
                    let env = ScopeEnv::new(None, Some(scope.vars.as_ref()), base);
                    match resolve_deferred_to_decls(property, raw_value, &env) {
                        Ok(_) => {
                            any_ok = true;
                            break;
                        }
                        Err(resolved) => failure = Some(resolved),
                    }
                }
            }

            if !any_ok {
                if let Some(resolved) = failure {
                    seen.insert(key);
                    record_dropped_declaration(
                        dropped,
                        "<deferred coverage>",
                        &format!("{property}: {resolved}"),
                    );
                }
            }
        }
    }
}

fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

fn collect_selector_text(parser: &mut Parser) -> Result<String, ()> {
    let start = parser.position();
    loop {
        match parser.next() {
            Ok(Token::CurlyBracketBlock) => {
                let slice = parser.slice_from(start);
                // cssparser's `Parser::next` skips whitespace and `/* ... */`
                // comments between tokens, so `position()` may be captured
                // before leading trivia. Strip comments from the raw slice
                // before passing it to the selector parser; otherwise a rule
                // like `/* nav */ .nav { ... }` ends up with a selector that
                // still contains the comment tokens (e.g. `* / nav / .nav`)
                // and never matches any element.
                let without_comments = strip_block_comments(slice);
                let selector_text =
                    without_comments.trim().trim_end_matches('{').trim().to_string();
                return Ok(selector_text);
            }
            Ok(_) => continue,
            Err(_) => return Err(()),
        }
    }
}

/// Strip `/* ... */` block comments from a CSS source fragment.
///
/// Keeps everything outside the comment markers verbatim, including
/// whitespace. Used on raw selector slices before tokenisation so that
/// `/* nav */ .nav` collapses to ` .nav`, avoiding spurious selector
/// parts. An unterminated `/*` drops everything from that point on, which
/// matches the cssparser tokenizer's tolerant behavior. Works at the char
/// level so it is safe for non-ASCII content inside comments.
fn strip_block_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut terminated = false;
            while let Some(inner) = chars.next() {
                if inner == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    terminated = true;
                    break;
                }
            }
            if !terminated {
                break;
            }
        } else {
            out.push(c);
        }
    }
    out
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
                    "first-of-type" => {
                        parts.push(SelectorPart::PseudoClass(PseudoClass::FirstOfType))
                    }
                    "last-of-type" => {
                        parts.push(SelectorPart::PseudoClass(PseudoClass::LastOfType))
                    }
                    // :root matches the root element; treat as universal for matching.
                    "root" => parts.push(SelectorPart::Universal),
                    "nth-child" if has_parens => {
                        let arg = consume_parenthesized(&mut chars);
                        if let Ok(n) = arg.trim().parse::<i32>() {
                            parts.push(SelectorPart::PseudoClass(PseudoClass::NthChild(n)));
                        } else {
                            return Err(());
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
                        } else {
                            return Err(());
                        }
                    }
                    _ => return Err(()),
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

fn parse_declaration(
    parser: &mut Parser,
    scope_hint: ScopeKey,
) -> Result<SmallVec<[StyleDeclaration; 2]>, ()> {
    let property = parser.expect_ident().map_err(|_| ())?.to_string();
    parser.expect_colon().map_err(|_| ())?;

    // Custom-property DEFINITIONS (`--name: value`) are collected separately
    // into `token_scopes` at parse time (see `collect_token_scopes`), so the
    // typed declaration path does not represent them. Consume the value to keep
    // the parser positioned at the next declaration and return no declarations —
    // crucially WITHOUT routing to `dropped`. Before Stage 3 these reached the
    // typed `match` below, failed every arm, and were recorded as dropped
    // (the historical "custom-property drop count"); now that var() resolution
    // is per-scope the definitions are live in the cascade, so dropping them
    // would be both wrong and noisy.
    if property.starts_with("--") {
        skip_to_semicolon(parser);
        return Ok(smallvec::smallvec![]);
    }

    // Fast path for values that carry a `var(`: capture the raw value verbatim
    // as a `Deferred` carrier instead of running the typed match (which has no
    // var() awareness). The concrete value is resolved per element and re-parsed
    // later in the cascade (see `apply_deferred_against_env`).
    //
    // This is BEFORE the typed match on purpose. To peek the value text without
    // committing the tokens to a typed parse, snapshot the parser state at the
    // start of the value, drain to the end of the declaration to slice the raw
    // value, then `reset` back so the typed match below sees an untouched value
    // on the (overwhelmingly common) no-`var(` path. Stage 3 deleted the global
    // resolve, so this branch is LIVE in production for every `var(`-bearing
    // value; the no-`var(` path keeps the byte-for-byte typed fast path.
    let value_state = parser.state();
    let value_start = parser.position();
    skip_to_semicolon(parser);
    let value_text = parser.slice_from(value_start);
    if contains_var_function(value_text) {
        let raw_value = value_text.trim().trim_end_matches(';').trim();
        return Ok(smallvec::smallvec![StyleDeclaration::Deferred {
            property: property.into_boxed_str(),
            raw_value: raw_value.into(),
            scope_hint,
        }]);
    }
    // No `var(`: rewind the value so the typed match re-reads it from scratch.
    parser.reset(&value_state);

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
                "normal" => JustifyContent::Normal,
                "flex-start" | "start" | "left" => JustifyContent::Start,
                "flex-end" | "end" | "right" => JustifyContent::End,
                "center" => JustifyContent::Center,
                "stretch" => JustifyContent::Stretch,
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
        "max-width" => StyleDeclaration::MaxWidth(parse_max_dimension(parser)?),
        "max-height" => StyleDeclaration::MaxHeight(parse_max_dimension(parser)?),
        "padding" => {
            let dims = expand_edge_dims(&parse_dimension_list(parser))?;
            match all_px_edges(dims) {
                // Pure-px keeps the f32 fast path (paint + transitions unchanged).
                Some(edges) => StyleDeclaration::Padding(edges),
                None => StyleDeclaration::PaddingDim(dims.map(Some)),
            }
        }
        "padding-top" => parse_padding_longhand(parser, 0)?,
        "padding-right" => parse_padding_longhand(parser, 1)?,
        "padding-bottom" => parse_padding_longhand(parser, 2)?,
        "padding-left" => parse_padding_longhand(parser, 3)?,
        "margin" => {
            let (edges, auto) = parse_margin_edges(parser)?;
            if auto.any() {
                StyleDeclaration::MarginWithAuto(edges, auto)
            } else {
                StyleDeclaration::Margin(edges)
            }
        }
        "margin-top" => parse_margin_longhand(parser, MarginSide::Top)?,
        "margin-right" => parse_margin_longhand(parser, MarginSide::Right)?,
        "margin-bottom" => parse_margin_longhand(parser, MarginSide::Bottom)?,
        "margin-left" => parse_margin_longhand(parser, MarginSide::Left)?,
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
            let v = match val.as_ref() {
                "visible" => Overflow::Visible,
                "hidden" => Overflow::Hidden,
                "scroll" | "auto" => Overflow::Scroll,
                _ => return Err(()),
            };
            // The `overflow` shorthand sets both axes. parse_rule's declaration
            // loop does not auto-drain on success, so an early return must consume
            // the terminating `;` or the next declaration is lost.
            let _ = parser.try_parse(cssparser::Parser::expect_semicolon);
            return Ok(smallvec::smallvec![
                StyleDeclaration::OverflowX(v),
                StyleDeclaration::OverflowY(v),
            ]);
        }
        "overflow-x" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::OverflowX(match val.as_ref() {
                "visible" => Overflow::Visible,
                "hidden" => Overflow::Hidden,
                "scroll" | "auto" => Overflow::Scroll,
                _ => return Err(()),
            })
        }
        "overflow-y" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::OverflowY(match val.as_ref() {
                "visible" => Overflow::Visible,
                "hidden" => Overflow::Hidden,
                "scroll" | "auto" => Overflow::Scroll,
                _ => return Err(()),
            })
        }
        "box-sizing" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::BoxSizing(match val.as_ref() {
                "content-box" => types::BoxSizing::ContentBox,
                "border-box" => types::BoxSizing::BorderBox,
                _ => return Err(()),
            })
        }
        "aspect-ratio" => {
            if parser
                .try_parse(|p| {
                    let ident = p.expect_ident().map_err(|_| ())?;
                    if ident.as_ref() == "auto" {
                        Ok(())
                    } else {
                        Err(())
                    }
                })
                .is_ok()
            {
                StyleDeclaration::AspectRatio(None)
            } else {
                let w: f32 = parser.expect_number().map_err(|_| ())?;
                let ratio = if parser.try_parse(|p| p.expect_delim('/')).is_ok() {
                    let h: f32 = parser.expect_number().map_err(|_| ())?;
                    w / h
                } else {
                    w
                };
                StyleDeclaration::AspectRatio(Some(ratio))
            }
        }
        "object-fit" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::ObjectFit(match val.as_ref() {
                "fill" => types::ObjectFit::Fill,
                "contain" => types::ObjectFit::Contain,
                "cover" => types::ObjectFit::Cover,
                "none" => types::ObjectFit::None,
                "scale-down" => types::ObjectFit::ScaleDown,
                _ => return Err(()),
            })
        }
        "object-position" => {
            fn keyword_to_pct(s: &str) -> Option<f32> {
                match s {
                    "left" | "top" => Some(0.0),
                    "center" => Some(50.0),
                    "right" | "bottom" => Some(100.0),
                    _ => std::option::Option::None,
                }
            }
            fn parse_pos_value(parser: &mut Parser) -> Result<f32, ()> {
                if let Ok(v) = parser.try_parse(|p| {
                    let id = p.expect_ident().map_err(|_| ())?;
                    keyword_to_pct(id.as_ref()).ok_or(())
                }) {
                    Ok(v)
                } else if let Ok(pct) = parser.try_parse(|p| p.expect_percentage().map_err(|_| ()))
                {
                    Ok(pct * 100.0)
                } else {
                    parser.expect_number().map_err(|_| ())
                }
            }
            let x = parse_pos_value(parser)?;
            let y = parse_pos_value(parser).unwrap_or(x);
            StyleDeclaration::ObjectPosition(types::ObjectPosition { x, y })
        }
        "background" => {
            // `background: none` clears the background. The default `Background`
            // is already transparent, so map it to a transparent color.
            if parser
                .try_parse(|p| {
                    let id = p.expect_ident().map_err(|_| ())?;
                    if id.eq_ignore_ascii_case("none") {
                        Ok(())
                    } else {
                        Err(())
                    }
                })
                .is_ok()
            {
                let _ = parser.try_parse(cssparser::Parser::expect_semicolon);
                return Ok(smallvec::smallvec![StyleDeclaration::Background(
                    types::Background::Color(Color::TRANSPARENT)
                )]);
            }

            let paint = match parser.try_parse(|p| parse_linear_gradient(p)) {
                Ok(gradient) => types::Background::LinearGradient(gradient),
                Err(_) => match parser.try_parse(|p| parse_radial_gradient(p)) {
                    Ok(gradient) => types::Background::RadialGradient(gradient),
                    Err(_) => types::Background::Color(parse_color(parser)?),
                },
            };
            // `ComputedStyle` holds a single background. For a comma-separated
            // multi-layer value, keep the first paintable layer and drain the
            // remaining layers (true N-layer paint is out of scope).
            while parser.try_parse(cssparser::Parser::expect_comma).is_ok() {
                // Consume the rest of this layer up to the next comma or end.
                while !parser.is_exhausted() {
                    let state = parser.state();
                    if parser.try_parse(cssparser::Parser::expect_comma).is_ok() {
                        parser.reset(&state);
                        break;
                    }
                    if parser.next().is_err() {
                        break;
                    }
                }
            }
            StyleDeclaration::Background(paint)
        }
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
        "border" => return parse_border_shorthand(parser, None),
        "border-top" => return parse_border_shorthand(parser, Some(BorderSide::Top)),
        "border-right" => return parse_border_shorthand(parser, Some(BorderSide::Right)),
        "border-bottom" => return parse_border_shorthand(parser, Some(BorderSide::Bottom)),
        "border-left" => return parse_border_shorthand(parser, Some(BorderSide::Left)),
        "border-color" => StyleDeclaration::BorderColor(parse_color(parser)?),
        "border-width" => StyleDeclaration::BorderWidth(parse_edges(parser)?),
        "border-top-width" => StyleDeclaration::BorderSideWidth(BorderSide::Top, parse_px(parser)?),
        "border-right-width" => {
            StyleDeclaration::BorderSideWidth(BorderSide::Right, parse_px(parser)?)
        }
        "border-bottom-width" => {
            StyleDeclaration::BorderSideWidth(BorderSide::Bottom, parse_px(parser)?)
        }
        "border-left-width" => {
            StyleDeclaration::BorderSideWidth(BorderSide::Left, parse_px(parser)?)
        }
        "border-top-color" => {
            StyleDeclaration::BorderSideColor(BorderSide::Top, parse_color(parser)?)
        }
        "border-right-color" => {
            StyleDeclaration::BorderSideColor(BorderSide::Right, parse_color(parser)?)
        }
        "border-bottom-color" => {
            StyleDeclaration::BorderSideColor(BorderSide::Bottom, parse_color(parser)?)
        }
        "border-left-color" => {
            StyleDeclaration::BorderSideColor(BorderSide::Left, parse_color(parser)?)
        }
        "border-radius" => {
            let corners = parse_corners_dim(parser)?;
            match all_px_corners(corners) {
                // Pure-px keeps the f32 fast path (paint + transitions unchanged).
                Some(px) => StyleDeclaration::BorderRadius(px),
                None => StyleDeclaration::BorderRadiusDim(corners),
            }
        }
        "border-style" => {
            // Standalone border-style. There is no dashed/dotted border
            // renderer, so non-`none` line styles are accepted-and-ignored.
            // `none`/`hidden` collapse the border to zero width.
            let val = parser.expect_ident().map_err(|_| ())?;
            match val.as_ref() {
                "none" | "hidden" => {
                    let _ = parser.try_parse(cssparser::Parser::expect_semicolon);
                    return Ok(smallvec::smallvec![StyleDeclaration::BorderWidth(Edges::all(0.0))]);
                }
                "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset"
                | "outset" => {
                    let _ = parser.try_parse(cssparser::Parser::expect_semicolon);
                    return Ok(SmallVec::new());
                }
                _ => return Err(()),
            }
        }
        "opacity" => StyleDeclaration::Opacity(parse_number(parser)?),
        "color" => StyleDeclaration::Color(parse_color(parser)?),
        "font" => return parse_font_shorthand(parser),
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
        "font-style" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            let style = match val.as_ref() {
                "normal" => FontStyle::Normal,
                "italic" => FontStyle::Italic,
                "oblique" => FontStyle::Oblique,
                _ => return Err(()),
            };
            StyleDeclaration::FontStyle(style)
        }
        "font-family" => {
            let family = parse_font_family_list(parser)?;
            StyleDeclaration::FontFamily(family)
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
        "text-transform" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::TextTransform(match val.as_ref() {
                "none" => TextTransform::None,
                "uppercase" => TextTransform::Uppercase,
                "lowercase" => TextTransform::Lowercase,
                "capitalize" => TextTransform::Capitalize,
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
        "text-overflow" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::TextOverflow(match val.as_ref() {
                "clip" => types::TextOverflow::Clip,
                "ellipsis" => types::TextOverflow::Ellipsis,
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
                "ns-resize" => CursorStyle::NsResize,
                "ew-resize" => CursorStyle::EwResize,
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
        "-webkit-app-region" | "app-region" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::AppRegion(match val.as_ref() {
                "auto" => AppRegion::Auto,
                "drag" => AppRegion::Drag,
                "no-drag" => AppRegion::NoDrag,
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
                "fixed" => CssPosition::Fixed,
                _ => return Err(()),
            })
        }
        "top" => StyleDeclaration::Top(parse_dimension(parser)?),
        "right" => StyleDeclaration::Right(parse_dimension(parser)?),
        "bottom" => StyleDeclaration::Bottom(parse_dimension(parser)?),
        "left" => StyleDeclaration::Left(parse_dimension(parser)?),
        "z-index" => {
            let val = parser.expect_integer().map_err(|_| ())?;
            StyleDeclaration::ZIndex(val)
        }
        "inset" => {
            // CSS `inset` shorthand: 1/2/3/4-value forms matching the spec.
            let values = parse_dimension_list(parser);
            let (top, right, bottom, left) = match values.len() {
                1 => (values[0], values[0], values[0], values[0]),
                2 => (values[0], values[1], values[0], values[1]),
                3 => (values[0], values[1], values[2], values[1]),
                4 => (values[0], values[1], values[2], values[3]),
                _ => return Err(()),
            };
            let _ = parser.try_parse(cssparser::Parser::expect_semicolon);
            return Ok(smallvec::smallvec![
                StyleDeclaration::Top(top),
                StyleDeclaration::Right(right),
                StyleDeclaration::Bottom(bottom),
                StyleDeclaration::Left(left),
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
            // Order-independent shorthand, mirroring `parse_border_shorthand`:
            // try width (px), color, then a style keyword that is swallowed.
            // `none`/`hidden` force width 0.
            let mut width = None;
            let mut color = None;
            let mut style_none = false;
            let mut consumed = false;

            while !parser.is_exhausted() {
                let state = parser.state();
                if parser.try_parse(cssparser::Parser::expect_semicolon).is_ok() {
                    parser.reset(&state);
                    break;
                }

                if width.is_none() {
                    if let Ok(w) = parser.try_parse(|p| parse_px(p)) {
                        width = Some(w);
                        consumed = true;
                        continue;
                    }
                }

                if color.is_none() {
                    if let Ok(c) = parser.try_parse(|p| parse_color(p)) {
                        color = Some(c);
                        consumed = true;
                        continue;
                    }
                }

                if let Ok(ident) = parser.try_parse(|p| p.expect_ident().map(|s| s.to_string())) {
                    match ident.as_str() {
                        "none" | "hidden" => {
                            style_none = true;
                            consumed = true;
                        }
                        "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset"
                        | "outset" => {
                            consumed = true;
                        }
                        _ => return Err(()),
                    }
                    continue;
                }

                return Err(());
            }

            if !consumed {
                return Err(());
            }

            let resolved_width = if style_none { 0.0 } else { width.unwrap_or(0.0) };
            let mut decls = SmallVec::<[StyleDeclaration; 2]>::new();
            decls.push(StyleDeclaration::OutlineWidth(resolved_width));
            if let Some(color) = color {
                decls.push(StyleDeclaration::OutlineColor(color));
            }

            let _ = parser.try_parse(cssparser::Parser::expect_semicolon);
            return Ok(decls);
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

        // CSS resize
        "resize" => {
            let val = parser.expect_ident().map_err(|_| ())?;
            StyleDeclaration::Resize(match val.as_ref() {
                "none" => types::CssResize::None,
                "both" => types::CssResize::Both,
                "horizontal" => types::CssResize::Horizontal,
                "vertical" => types::CssResize::Vertical,
                _ => return Err(()),
            })
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

        // CSS `transform`: a `translate*` / `scale*` / `rotate` function
        // list, or the `none` keyword. Unsupported forms (`matrix`, `skew`,
        // 3D functions) return an error so the cascade drops only this
        // declaration while other declarations on the same selector still
        // apply. See `parse_transform`.
        "transform" => match parse_transform(parser) {
            Some(t) => StyleDeclaration::Transform(t),
            None => return Err(()),
        },

        // CSS `mask-image`. Only the `linear-gradient(...)` branch is
        // supported. `none`, `url()`, and other image sources parse to an
        // error today. The underlying gradient parser is the same one
        // used by `background: linear-gradient(...)`.
        "mask-image" => {
            let gradient = parse_mask_image(parser)?;
            StyleDeclaration::MaskImage(gradient)
        }

        // `text-shadow`: `none` (empty list) or one or more comma-separated
        // glow layers. Painted as a colored blur behind the text.
        "text-shadow" => StyleDeclaration::TextShadowList(parse_text_shadow_list(parser)?),

        // Inert no-op accepts: recognized-but-ignored properties that have no
        // render target in this engine. Accepting them is honest CSS
        // forward-compat (mirrors the `backdrop-filter: none` no-op). Each
        // validates that at least one token is present, then drains to `;`.
        "appearance"
        | "-webkit-appearance"
        | "-webkit-font-smoothing"
        | "border-collapse"
        | "background-repeat"
        | "font-feature-settings"
        | "font-variant-numeric"
        | "scrollbar-width" => {
            // Require a value so an empty declaration still errors, then drain
            // ONLY to this declaration's terminator (not the rest of the block).
            parser.next().map_err(|_| ())?;
            while let Ok(token) = parser.next() {
                if matches!(token, Token::Semicolon) {
                    break;
                }
            }
            return Ok(SmallVec::new());
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

fn parse_font_shorthand(parser: &mut Parser) -> Result<SmallVec<[StyleDeclaration; 2]>, ()> {
    let mut font_weight = None;
    let mut font_size = None;
    let mut line_height = None;
    let mut font_family = None;

    while !parser.is_exhausted() {
        let state = parser.state();
        if parser.try_parse(cssparser::Parser::expect_semicolon).is_ok() {
            parser.reset(&state);
            break;
        }

        if font_size.is_none() {
            if let Ok((size, lh)) = parser.try_parse(|p| parse_font_size_and_line_height(p)) {
                font_size = Some(size);
                line_height = lh;
                continue;
            }
            if font_weight.is_none() {
                if let Ok(weight) = parser.try_parse(|p| parse_font_weight_value(p)) {
                    font_weight = Some(weight);
                    continue;
                }
            }
            if parser.try_parse(|p| parse_ignored_font_keyword(p)).is_ok() {
                continue;
            }
            return Err(());
        }

        if font_family.is_none() {
            font_family = Some(parse_font_family_list(parser)?);
            break;
        }

        break;
    }

    let size = font_size.ok_or(())?;
    let family = font_family.ok_or(())?;
    let mut decls = SmallVec::<[StyleDeclaration; 2]>::new();
    if let Some(weight) = font_weight {
        decls.push(StyleDeclaration::FontWeight(weight));
    }
    decls.push(StyleDeclaration::FontSize(size));
    if let Some(lh) = line_height {
        decls.push(StyleDeclaration::LineHeight(lh));
    }
    decls.push(StyleDeclaration::FontFamily(family));

    Ok(decls)
}

fn parse_font_weight_value(parser: &mut Parser) -> Result<FontWeight, ()> {
    let tok = parser.next().map_err(|_| ())?.clone();
    match tok {
        Token::Ident(ref s) => match s.as_ref() {
            "normal" => Ok(FontWeight::Normal),
            "bold" => Ok(FontWeight::Bold),
            _ => Err(()),
        },
        Token::Number { int_value: Some(n), .. } => Ok(FontWeight::W(n as u16)),
        Token::Number { value, .. } if value.fract() == 0.0 => Ok(FontWeight::W(value as u16)),
        _ => Err(()),
    }
}

fn parse_ignored_font_keyword(parser: &mut Parser) -> Result<(), ()> {
    let ident = parser.expect_ident().map_err(|_| ())?;
    match ident.as_ref() {
        "normal" | "italic" | "oblique" | "small-caps" => Ok(()),
        _ => Err(()),
    }
}

fn parse_font_size_and_line_height(parser: &mut Parser) -> Result<(f32, Option<f32>), ()> {
    let size = parse_font_size_px(parser)?;
    let line_height = if parser.try_parse(|p| p.expect_delim('/')).is_ok() {
        Some(parse_font_line_height(parser, size)?)
    } else {
        None
    };
    Ok((size, line_height))
}

fn parse_font_size_px(parser: &mut Parser) -> Result<f32, ()> {
    match parser.next().map_err(|_| ())? {
        Token::Dimension { value, unit, .. } => {
            if unit.as_ref() == "vh" || unit.as_ref() == "vw" {
                return Err(());
            }
            Ok(*value)
        }
        Token::Number { value, .. } if *value == 0.0 => Ok(*value),
        _ => Err(()),
    }
}

fn parse_font_line_height(parser: &mut Parser, font_size: f32) -> Result<f32, ()> {
    match parser.next().map_err(|_| ())? {
        Token::Number { value, .. } => Ok(*value),
        Token::Percentage { unit_value, .. } => Ok(*unit_value),
        Token::Dimension { value, unit, .. } => {
            if unit.as_ref() == "vh" || unit.as_ref() == "vw" || font_size <= 0.0 {
                return Err(());
            }
            Ok(*value / font_size)
        }
        Token::Ident(ref s) if s.as_ref() == "normal" => Ok(1.2),
        _ => Err(()),
    }
}

fn parse_font_family_list(parser: &mut Parser) -> Result<String, ()> {
    let mut families = Vec::new();
    let mut current = String::new();
    let mut consumed = false;

    while !parser.is_exhausted() {
        let state = parser.state();
        if parser.try_parse(cssparser::Parser::expect_semicolon).is_ok() {
            break;
        }
        parser.reset(&state);

        match parser.next().map_err(|_| ())? {
            Token::QuotedString(s) | Token::Ident(s) => {
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(s.as_ref());
                consumed = true;
            }
            Token::Comma => {
                let family = current.trim();
                if !family.is_empty() {
                    families.push(family.to_string());
                    current.clear();
                }
            }
            _ => return Err(()),
        }
    }

    let family = current.trim();
    if !family.is_empty() {
        families.push(family.to_string());
    }

    if consumed && !families.is_empty() {
        Ok(families.join(", "))
    } else {
        Err(())
    }
}

fn parse_border_shorthand(
    parser: &mut Parser,
    side: Option<BorderSide>,
) -> Result<SmallVec<[StyleDeclaration; 2]>, ()> {
    let mut width = None;
    let mut color = None;
    let mut style_none = false;
    let mut consumed = false;

    while !parser.is_exhausted() {
        let state = parser.state();
        if parser.try_parse(cssparser::Parser::expect_semicolon).is_ok() {
            parser.reset(&state);
            break;
        }

        if width.is_none() {
            if let Ok(w) = parser.try_parse(|p| parse_px(p)) {
                width = Some(w);
                consumed = true;
                continue;
            }
        }

        if color.is_none() {
            if let Ok(c) = parser.try_parse(|p| parse_color(p)) {
                color = Some(c);
                consumed = true;
                continue;
            }
        }

        if let Ok(ident) = parser.try_parse(|p| p.expect_ident().map(|s| s.to_string())) {
            match ident.as_str() {
                "none" | "hidden" => {
                    style_none = true;
                    consumed = true;
                }
                "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset"
                | "outset" => {
                    consumed = true;
                }
                _ => return Err(()),
            }
            continue;
        }

        return Err(());
    }

    if !consumed {
        return Err(());
    }

    let resolved_width = if style_none { 0.0 } else { width.ok_or(())? };
    let mut decls = SmallVec::<[StyleDeclaration; 2]>::new();
    if let Some(side) = side {
        decls.push(StyleDeclaration::BorderSideWidth(side, resolved_width));
    } else {
        decls.push(StyleDeclaration::BorderWidth(Edges::all(resolved_width)));
    }
    if let Some(color) = color {
        decls.push(StyleDeclaration::BorderColor(color));
    }

    let _ = parser.try_parse(cssparser::Parser::expect_semicolon);
    Ok(decls)
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

/// A `calc()` expression reduced to a linear combination of a constant and
/// relative units. Length-valued calc resolves to `px + percent%·container +
/// vw·viewport_w/100 + vh·viewport_h/100`.
#[derive(Clone, Copy, Default)]
struct CalcTerms {
    px: f32,
    percent: f32,
    vw: f32,
    vh: f32,
}

impl CalcTerms {
    fn add(self, o: CalcTerms) -> CalcTerms {
        CalcTerms {
            px: self.px + o.px,
            percent: self.percent + o.percent,
            vw: self.vw + o.vw,
            vh: self.vh + o.vh,
        }
    }
    fn sub(self, o: CalcTerms) -> CalcTerms {
        CalcTerms {
            px: self.px - o.px,
            percent: self.percent - o.percent,
            vw: self.vw - o.vw,
            vh: self.vh - o.vh,
        }
    }
    fn scale(self, s: f32) -> CalcTerms {
        CalcTerms { px: self.px * s, percent: self.percent * s, vw: self.vw * s, vh: self.vh * s }
    }
}

/// A value inside `calc()`: either a dimensionless number or a length. The CSS
/// calc type algebra: `length ± length`, `length × number`, `length ÷ number`,
/// `number op number`; `length × length`, `length + number`, etc. are invalid.
enum CalcVal {
    Num(f32),
    Len(CalcTerms),
}

/// Parse `calc( <sum> )` into a length `CalcTerms`. Errors (so the caller drops
/// or falls through) on a non-`calc` token, a malformed expression, an
/// unsupported unit (e.g. `em`), or a result that is a bare number rather than
/// a length. Always invoked under `try_parse`, so a partial consume is rolled
/// back by the caller.
fn parse_calc_terms(parser: &mut Parser) -> Result<CalcTerms, ()> {
    let name = parser.expect_function().map_err(|_| ())?;
    if !name.as_ref().eq_ignore_ascii_case("calc") {
        return Err(());
    }
    parser
        .parse_nested_block(|p| -> Result<CalcTerms, cssparser::ParseError<'_, ()>> {
            let val = calc_sum(p)?;
            p.expect_exhausted()?;
            match val {
                CalcVal::Len(t) => Ok(t),
                CalcVal::Num(_) => Err(p.new_custom_error(())),
            }
        })
        .map_err(|_| ())
}

/// `<product> ( ('+' | '-') <product> )*`
fn calc_sum<'i>(p: &mut Parser<'i, '_>) -> Result<CalcVal, cssparser::ParseError<'i, ()>> {
    let mut acc = calc_product(p)?;
    loop {
        let op = p.try_parse(|p| match p.next()? {
            Token::Delim('+') => Ok('+'),
            Token::Delim('-') => Ok('-'),
            _ => Err(p.new_custom_error::<(), ()>(())),
        });
        match op {
            Ok('+') => acc = calc_combine(acc, calc_product(p)?, '+', p)?,
            Ok('-') => acc = calc_combine(acc, calc_product(p)?, '-', p)?,
            _ => break,
        }
    }
    Ok(acc)
}

/// `<value> ( ('*' | '/') <value> )*`
fn calc_product<'i>(p: &mut Parser<'i, '_>) -> Result<CalcVal, cssparser::ParseError<'i, ()>> {
    let mut acc = calc_value(p)?;
    loop {
        let op = p.try_parse(|p| match p.next()? {
            Token::Delim('*') => Ok('*'),
            Token::Delim('/') => Ok('/'),
            _ => Err(p.new_custom_error::<(), ()>(())),
        });
        match op {
            Ok('*') => acc = calc_combine(acc, calc_value(p)?, '*', p)?,
            Ok('/') => acc = calc_combine(acc, calc_value(p)?, '/', p)?,
            _ => break,
        }
    }
    Ok(acc)
}

/// A length token, a number, a parenthesized sub-expression, or a nested
/// `calc()`.
fn calc_value<'i>(p: &mut Parser<'i, '_>) -> Result<CalcVal, cssparser::ParseError<'i, ()>> {
    match p.next()?.clone() {
        Token::Number { value, .. } => Ok(CalcVal::Num(value)),
        Token::Dimension { value, ref unit, .. } => {
            let u = unit.as_ref();
            if u.eq_ignore_ascii_case("px") {
                Ok(CalcVal::Len(CalcTerms { px: value, ..Default::default() }))
            } else if u.eq_ignore_ascii_case("vw") {
                Ok(CalcVal::Len(CalcTerms { vw: value, ..Default::default() }))
            } else if u.eq_ignore_ascii_case("vh") {
                Ok(CalcVal::Len(CalcTerms { vh: value, ..Default::default() }))
            } else {
                Err(p.new_custom_error(()))
            }
        }
        Token::Percentage { unit_value, .. } => {
            Ok(CalcVal::Len(CalcTerms { percent: unit_value * 100.0, ..Default::default() }))
        }
        Token::ParenthesisBlock => p.parse_nested_block(calc_sum),
        Token::Function(ref name) if name.eq_ignore_ascii_case("calc") => {
            p.parse_nested_block(calc_sum)
        }
        _ => Err(p.new_custom_error(())),
    }
}

/// Apply a binary calc operator, enforcing the calc type algebra.
fn calc_combine<'i>(
    a: CalcVal,
    b: CalcVal,
    op: char,
    p: &Parser<'i, '_>,
) -> Result<CalcVal, cssparser::ParseError<'i, ()>> {
    let err = || p.new_custom_error(());
    Ok(match op {
        '+' | '-' => {
            let sign = if op == '+' { 1.0 } else { -1.0 };
            match (a, b) {
                (CalcVal::Len(x), CalcVal::Len(y)) => {
                    CalcVal::Len(if sign > 0.0 { x.add(y) } else { x.sub(y) })
                }
                (CalcVal::Num(x), CalcVal::Num(y)) => CalcVal::Num(x + sign * y),
                _ => return Err(err()),
            }
        }
        '*' => match (a, b) {
            (CalcVal::Num(x), CalcVal::Num(y)) => CalcVal::Num(x * y),
            (CalcVal::Len(t), CalcVal::Num(n)) | (CalcVal::Num(n), CalcVal::Len(t)) => {
                CalcVal::Len(t.scale(n))
            }
            _ => return Err(err()), // length × length is invalid
        },
        '/' => match (a, b) {
            (CalcVal::Num(x), CalcVal::Num(y)) if y != 0.0 => CalcVal::Num(x / y),
            (CalcVal::Len(t), CalcVal::Num(n)) if n != 0.0 => CalcVal::Len(t.scale(1.0 / n)),
            // Division by zero or by a length is invalid.
            _ => return Err(err()),
        },
        _ => return Err(err()),
    })
}

/// Interpret a parsed `calc()` as a [`Dimension`]. Pure-`px` collapses to
/// `Px`, pure-`percent` to `Percent`; a `px`/`vw`/`vh` mix becomes
/// `Dimension::Calc`. A `percent` term mixed with any other term is rejected —
/// taffy cannot represent `length + percent`, and the app authors no such form
/// on a supported property.
fn calc_terms_to_dimension(t: CalcTerms) -> Result<Dimension, ()> {
    let has_relative = t.vw != 0.0 || t.vh != 0.0;
    if t.percent != 0.0 {
        if t.px != 0.0 || has_relative {
            return Err(());
        }
        return Ok(Dimension::Percent(t.percent));
    }
    if has_relative {
        Ok(Dimension::Calc { px: t.px, vw: t.vw, vh: t.vh })
    } else {
        Ok(Dimension::Px(t.px))
    }
}

fn parse_px(parser: &mut Parser) -> Result<f32, ()> {
    // `calc()` that reduces to a constant px (e.g. `calc(var(--sp-3) * -1)`
    // after var resolution). Relative units can't become a constant on the px
    // pathway, so reject them here.
    if let Ok(t) = parser.try_parse(|p| parse_calc_terms(p)) {
        if t.percent == 0.0 && t.vw == 0.0 && t.vh == 0.0 {
            return Ok(t.px);
        }
        return Err(());
    }
    match parser.next().map_err(|_| ())? {
        Token::Dimension { value, unit, .. } => {
            // Viewport relative units cannot resolve without layout context.
            // Reject `vh`/`vw` here so properties that use the px pathway
            // (padding, border-width, gap, etc.) error loudly instead of
            // silently treating `5vh` as `5px`. Broadening the pathway to
            // accept viewport units is tracked as a framework gap.
            if unit.as_ref() == "vh" || unit.as_ref() == "vw" {
                return Err(());
            }
            Ok(*value)
        }
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
    if let Ok(t) = parser.try_parse(|p| parse_calc_terms(p)) {
        return calc_terms_to_dimension(t);
    }
    match parser.next().map_err(|_| ())? {
        Token::Ident(ref s) if s.as_ref() == "auto" => Ok(Dimension::Auto),
        Token::Dimension { value, unit, .. } => match unit.as_ref() {
            "%" => Ok(Dimension::Percent(*value)),
            "vh" => Ok(Dimension::Vh(*value)),
            "vw" => Ok(Dimension::Vw(*value)),
            _ => Ok(Dimension::Px(*value)),
        },
        Token::Percentage { unit_value, .. } => Ok(Dimension::Percent(*unit_value * 100.0)),
        Token::Number { value, .. } => Ok(Dimension::Px(*value)),
        _ => Err(()),
    }
}

fn parse_max_dimension(parser: &mut Parser) -> Result<Dimension, ()> {
    if parser
        .try_parse(|p| match p.next().map_err(|_| ())? {
            Token::Ident(ref s) if s.as_ref().eq_ignore_ascii_case("none") => Ok(()),
            _ => Err(()),
        })
        .is_ok()
    {
        return Ok(Dimension::Auto);
    }

    parse_dimension(parser)
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

fn parse_dimension_list(parser: &mut Parser) -> Vec<Dimension> {
    let mut values = Vec::with_capacity(4);
    while values.len() < 4 {
        match parser.try_parse(|p| parse_dimension(p)) {
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

/// Expand a CSS 1–4 value edge list into `[top, right, bottom, left]`.
fn expand_edge_dims(dims: &[Dimension]) -> Result<[Dimension; 4], ()> {
    Ok(match *dims {
        [a] => [a, a, a, a],
        [a, b] => [a, b, a, b],
        [a, b, c] => [a, b, c, b],
        [a, b, c, d] => [a, b, c, d],
        _ => return Err(()),
    })
}

/// If every edge is `px`, collapse to a resolved `Edges` (the f32 fast path);
/// otherwise `None`, so the caller emits the unit-preserving `PaddingDim`.
fn all_px_edges(d: [Dimension; 4]) -> Option<Edges> {
    match d {
        [Dimension::Px(top), Dimension::Px(right), Dimension::Px(bottom), Dimension::Px(left)] => {
            Some(Edges { top, right, bottom, left })
        }
        _ => None,
    }
}

/// Parse one padding longhand value: `px` keeps the f32 fast-path variant
/// (preserving transition behavior); any viewport/percent unit becomes a
/// single-side `PaddingDim`. `side` is the `[top, right, bottom, left]` index.
fn parse_padding_longhand(parser: &mut Parser, side: usize) -> Result<StyleDeclaration, ()> {
    Ok(match parse_dimension(parser)? {
        Dimension::Px(v) => match side {
            0 => StyleDeclaration::PaddingTop(v),
            1 => StyleDeclaration::PaddingRight(v),
            2 => StyleDeclaration::PaddingBottom(v),
            _ => StyleDeclaration::PaddingLeft(v),
        },
        other => {
            let mut arr = [None; 4];
            arr[side] = Some(other);
            StyleDeclaration::PaddingDim(arr)
        }
    })
}

#[derive(Clone, Copy)]
enum MarginSide {
    Top,
    Right,
    Bottom,
    Left,
}

fn parse_margin_longhand(parser: &mut Parser, side: MarginSide) -> Result<StyleDeclaration, ()> {
    let (value, is_auto) = parse_margin_value(parser)?;
    Ok(match (side, is_auto) {
        (MarginSide::Top, false) => StyleDeclaration::MarginTop(value),
        (MarginSide::Right, false) => StyleDeclaration::MarginRight(value),
        (MarginSide::Bottom, false) => StyleDeclaration::MarginBottom(value),
        (MarginSide::Left, false) => StyleDeclaration::MarginLeft(value),
        (MarginSide::Top, true) => StyleDeclaration::MarginTopAuto,
        (MarginSide::Right, true) => StyleDeclaration::MarginRightAuto,
        (MarginSide::Bottom, true) => StyleDeclaration::MarginBottomAuto,
        (MarginSide::Left, true) => StyleDeclaration::MarginLeftAuto,
    })
}

fn parse_margin_edges(parser: &mut Parser) -> Result<(Edges, EdgeAutoFlags), ()> {
    let mut values = Vec::with_capacity(4);
    while values.len() < 4 {
        match parser.try_parse(|p| parse_margin_value(p)) {
            Ok(v) => values.push(v),
            Err(_) => break,
        }
    }

    let (top, right, bottom, left) = match values.as_slice() {
        [a] => (*a, *a, *a, *a),
        [a, b] => (*a, *b, *a, *b),
        [a, b, c] => (*a, *b, *c, *b),
        [a, b, c, d] => (*a, *b, *c, *d),
        _ => return Err(()),
    };

    Ok((
        Edges { top: top.0, right: right.0, bottom: bottom.0, left: left.0 },
        EdgeAutoFlags { top: top.1, right: right.1, bottom: bottom.1, left: left.1 },
    ))
}

fn parse_margin_value(parser: &mut Parser) -> Result<(f32, bool), ()> {
    if parser
        .try_parse(|p| match p.next().map_err(|_| ())? {
            Token::Ident(ref s) if s.as_ref().eq_ignore_ascii_case("auto") => Ok(()),
            _ => Err(()),
        })
        .is_ok()
    {
        return Ok((0.0, true));
    }
    parse_px(parser).map(|v| (v, false))
}

/// Parse a `border-radius` value as a unit-preserving `CornersDim`, accepting
/// both `px` and `%` corners. Applies the CSS 1/2/4-value expansion
/// (`tl tr br bl`). The dispatch site collapses a pure-px result back to the
/// f32 `Corners` fast path so transitions and DPI scaling are unchanged.
fn parse_corners_dim(parser: &mut Parser) -> Result<CornersDim, ()> {
    let mut values: Vec<LengthOrPercent> = Vec::with_capacity(4);
    while values.len() < 4 {
        match parser.try_parse(parse_length_or_percent) {
            Ok(v) => values.push(v),
            Err(_) => break,
        }
    }
    match values.as_slice() {
        [a] => Ok(CornersDim { top_left: *a, top_right: *a, bottom_right: *a, bottom_left: *a }),
        [a, b] => Ok(CornersDim { top_left: *a, top_right: *b, bottom_right: *a, bottom_left: *b }),
        [a, b, c, d] => {
            Ok(CornersDim { top_left: *a, top_right: *b, bottom_right: *c, bottom_left: *d })
        }
        _ => Err(()),
    }
}

/// Collapse a `CornersDim` to the resolved f32 `Corners` when every corner is
/// `px`; otherwise `None`, so the caller emits the unit-preserving
/// `BorderRadiusDim`.
fn all_px_corners(c: CornersDim) -> Option<Corners> {
    match c {
        CornersDim {
            top_left: LengthOrPercent::Px(top_left),
            top_right: LengthOrPercent::Px(top_right),
            bottom_right: LengthOrPercent::Px(bottom_right),
            bottom_left: LengthOrPercent::Px(bottom_left),
        } => Some(Corners { top_left, top_right, bottom_right, bottom_left }),
        _ => None,
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
            // Optional leading `<angle>,` or `to <side>[ <side>],`. If both
            // are absent, CSS defaults to 180deg (gradient flows top to
            // bottom, first stop at the top). `to <side>` is the CSS Images
            // Level 3 side based form that is commonly used for
            // `mask-image: linear-gradient(to right, ...)`.
            let angle_deg = p
                .try_parse(|p| -> Result<f32, ()> {
                    match p.next() {
                        Ok(Token::Dimension { value, unit, .. })
                            if unit.as_ref().eq_ignore_ascii_case("deg") =>
                        {
                            let v = *value;
                            p.expect_comma().map_err(|_| ())?;
                            Ok(v)
                        }
                        Ok(Token::Ident(name)) if name.as_ref().eq_ignore_ascii_case("to") => {
                            // Consume one or two side keywords.
                            let a = p.expect_ident_cloned().map_err(|_| ())?;
                            let b = p.try_parse(|p| p.expect_ident_cloned()).ok();
                            let degrees =
                                sides_to_angle_deg(a.as_ref(), b.as_ref().map(|s| s.as_ref()))
                                    .ok_or(())?;
                            p.expect_comma().map_err(|_| ())?;
                            Ok(degrees)
                        }
                        _ => Err(()),
                    }
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

/// Translate CSS `to <side>[ <side>]` into a gradient angle in degrees.
///
/// The mapping follows CSS Images Level 3: `to top` is 0deg, `to right` is
/// 90deg, `to bottom` is 180deg, `to left` is 270deg. Corners average the
/// two adjacent side angles: `to top right` is 45deg, `to bottom right`
/// 135deg, etc. Returns `None` on unknown or conflicting keywords.
fn sides_to_angle_deg(a: &str, b: Option<&str>) -> Option<f32> {
    fn side_deg(s: &str) -> Option<f32> {
        match s.to_ascii_lowercase().as_str() {
            "top" => Some(0.0),
            "right" => Some(90.0),
            "bottom" => Some(180.0),
            "left" => Some(270.0),
            _ => None,
        }
    }
    match b {
        None => side_deg(a),
        Some(b_str) => {
            // Accept vertical + horizontal in either order.
            let vertical = matches!(a.to_ascii_lowercase().as_str(), "top" | "bottom")
                || matches!(b_str.to_ascii_lowercase().as_str(), "top" | "bottom");
            let horizontal = matches!(a.to_ascii_lowercase().as_str(), "left" | "right")
                || matches!(b_str.to_ascii_lowercase().as_str(), "left" | "right");
            if !(vertical && horizontal) {
                return None;
            }
            // Corner angles: top right=45, bottom right=135, bottom left=225,
            // top left=315.
            let is_top = matches!(a.to_ascii_lowercase().as_str(), "top")
                || matches!(b_str.to_ascii_lowercase().as_str(), "top");
            let is_right = matches!(a.to_ascii_lowercase().as_str(), "right")
                || matches!(b_str.to_ascii_lowercase().as_str(), "right");
            match (is_top, is_right) {
                (true, true) => Some(45.0),
                (false, true) => Some(135.0),
                (false, false) => Some(225.0),
                (true, false) => Some(315.0),
            }
        }
    }
}

/// Parse a `mask-image: <linear-gradient>` declaration. Accepts
/// `linear-gradient(...)` and `repeating-linear-gradient(...)`. Other mask
/// sources (`url(...)`, `image(...)`, `none`) are rejected so callers can
/// cascade in the fallback.
fn parse_mask_image(parser: &mut Parser) -> Result<types::LinearGradient, ()> {
    // Reuse the existing gradient parser verbatim. The gradient grammar is
    // identical in `background-image` and `mask-image` contexts.
    parse_linear_gradient(parser)
}

/// Parse a CSS `transform` value into a decomposed [`types::Transform`].
///
/// Accepts the `none` keyword (returns [`types::Transform::IDENTITY`], which
/// still overrides a less-specific transform in the cascade) and a
/// whitespace-separated list of the functions the app stylesheet uses:
/// `translateX`/`translateY`/`translate`, `scale`/`scaleX`/`scaleY`, and
/// `rotate`. Returns `None` (so the caller drops the declaration and the
/// coverage guardrail flags it) when ANY function in the list is unsupported
/// (`matrix`, `skew`, 3D, ...) or malformed — partial application of a list
/// would silently mis-render, so the whole declaration is rejected.
///
/// Components accumulate across the list: later `translate*` overwrite the
/// matching axis, `scale*` overwrite the matching axis, and `rotate` adds.
/// The renderer composes them in the canonical `Translate · Rotate · Scale`
/// order, which matches the only multi-function form authored
/// (`translateY(..) scale(..)`).
fn parse_transform(parser: &mut Parser) -> Option<types::Transform> {
    let mut t = types::Transform::IDENTITY;
    let mut saw_any = false;

    loop {
        // Pull the next item: the `none` keyword (an explicit identity that
        // still overrides a less-specific transform), a transform function, or
        // EOF. `next()` returns `Err` at the end of the value, ending the list.
        // The token borrow is released as the match yields the owned name.
        let fn_name = match parser.next() {
            Ok(Token::Ident(id)) if id.as_ref().eq_ignore_ascii_case("none") => {
                return Some(types::Transform::IDENTITY);
            }
            Ok(Token::Function(name)) => name.clone(),
            // The declaration value parser includes the trailing `;`; that (or
            // EOF) ends the function list.
            Ok(Token::Semicolon) => break,
            // Any other token means the value is not a clean function list —
            // reject so the coverage guardrail surfaces it.
            Ok(_) => return None,
            Err(_) => break,
        };
        saw_any = true;

        // Each function is its own nested block. On an unsupported or
        // malformed function we still must drain the block to keep the outer
        // parser's `()` balanced, then reject the whole declaration.
        let parsed = parser.parse_nested_block(|p| -> Result<(), cssparser::ParseError<'_, ()>> {
            let name = fn_name.as_ref();
            if name.eq_ignore_ascii_case("translatex") {
                t.translate_x = Some(parse_translate_len(p)?);
            } else if name.eq_ignore_ascii_case("translatey") {
                t.translate_y = Some(parse_translate_len(p)?);
            } else if name.eq_ignore_ascii_case("translate") {
                // `translate(tx)` or `translate(tx, ty)`.
                t.translate_x = Some(parse_translate_len(p)?);
                if p.try_parse(|p| p.expect_comma()).is_ok() {
                    t.translate_y = Some(parse_translate_len(p)?);
                }
            } else if name.eq_ignore_ascii_case("scale") {
                // `scale(s)` uniform, or `scale(sx, sy)`.
                let sx = parse_scale_factor(p)?;
                let sy = if p.try_parse(|p| p.expect_comma()).is_ok() {
                    parse_scale_factor(p)?
                } else {
                    sx
                };
                t.scale_x = sx;
                t.scale_y = sy;
            } else if name.eq_ignore_ascii_case("scalex") {
                t.scale_x = parse_scale_factor(p)?;
            } else if name.eq_ignore_ascii_case("scaley") {
                t.scale_y = parse_scale_factor(p)?;
            } else if name.eq_ignore_ascii_case("rotate") {
                t.rotate += parse_angle_rad(p)?;
            } else {
                // Unsupported function (matrix, skew, perspective, 3D...).
                drain_tokens(p);
                return Err(p.new_custom_error(()));
            }
            Ok(())
        });
        parsed.ok()?;
    }

    if saw_any {
        Some(t)
    } else {
        None
    }
}

/// Parse a `<length-percentage>` translate argument (`px`, `%`, or a bare
/// `0`) into a [`types::TransformX`].
fn parse_translate_len<'i>(
    p: &mut Parser<'i, '_>,
) -> Result<types::TransformX, cssparser::ParseError<'i, ()>> {
    match p.next() {
        Ok(Token::Percentage { unit_value, .. }) => Ok(types::TransformX::Percent(*unit_value)),
        Ok(Token::Dimension { value, unit, .. }) if unit.as_ref().eq_ignore_ascii_case("px") => {
            Ok(types::TransformX::Px(*value))
        }
        Ok(Token::Number { value, .. }) if *value == 0.0 => Ok(types::TransformX::Px(0.0)),
        _ => Err(p.new_custom_error(())),
    }
}

/// Parse a unitless `<number>` (a `scale` factor).
fn parse_scale_factor<'i>(p: &mut Parser<'i, '_>) -> Result<f32, cssparser::ParseError<'i, ()>> {
    match p.next() {
        Ok(Token::Number { value, .. }) => Ok(*value),
        _ => Err(p.new_custom_error(())),
    }
}

/// Parse an `<angle>` (`deg`, `rad`, `grad`, `turn`, or a bare `0`) into
/// radians.
fn parse_angle_rad<'i>(p: &mut Parser<'i, '_>) -> Result<f32, cssparser::ParseError<'i, ()>> {
    use std::f32::consts::PI;
    match p.next() {
        Ok(Token::Dimension { value, unit, .. }) => {
            let u = unit.as_ref();
            if u.eq_ignore_ascii_case("deg") {
                Ok(value.to_radians())
            } else if u.eq_ignore_ascii_case("rad") {
                Ok(*value)
            } else if u.eq_ignore_ascii_case("grad") {
                Ok(value * PI / 200.0)
            } else if u.eq_ignore_ascii_case("turn") {
                Ok(value * 2.0 * PI)
            } else {
                Err(p.new_custom_error(()))
            }
        }
        Ok(Token::Number { value, .. }) if *value == 0.0 => Ok(0.0),
        _ => Err(p.new_custom_error(())),
    }
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
        Token::Function(ref name) if name.as_ref() == "oklch" => parser
            .parse_nested_block(|p| parse_oklch_body(p).map_err(|_| p.new_custom_error(())))
            .map_err(|_: cssparser::ParseError<'_, ()>| ()),
        _ => Err(()),
    }
}

/// Parse the body of an `oklch(L C H)` or `oklch(L C H / A)` function.
///
/// Grammar (CSS Color Level 4):
/// * L: number `0.0..=1.0` or percentage `0%..=100%` (percentage of 1.0)
/// * C: number (clamped to `>= 0.0`) or percentage (percentage of `0.4`)
/// * H: number in degrees, or `<angle>` (deg/grad/rad/turn)
/// * A: optional, separated by `/`; number `0.0..=1.0` or percentage
fn parse_oklch_body(parser: &mut Parser) -> Result<Color, ()> {
    let lightness = parse_oklch_lightness(parser)?;
    let chroma = parse_oklch_chroma(parser)?;
    let hue_rad = parse_oklch_hue(parser)?;
    let alpha = if parser.try_parse(|p| p.expect_delim('/')).is_ok() {
        parse_alpha_unit(parser)?
    } else {
        1.0
    };

    let a = chroma * hue_rad.cos();
    let b = chroma * hue_rad.sin();
    Ok(crate::style::transition::oklab_to_srgb(lightness, a, b, alpha))
}

fn parse_oklch_lightness(parser: &mut Parser) -> Result<f32, ()> {
    match parser.next().map_err(|_| ())? {
        Token::Number { value, .. } => Ok(value.clamp(0.0, 1.0)),
        Token::Percentage { unit_value, .. } => Ok(unit_value.clamp(0.0, 1.0)),
        _ => Err(()),
    }
}

fn parse_oklch_chroma(parser: &mut Parser) -> Result<f32, ()> {
    match parser.next().map_err(|_| ())? {
        Token::Number { value, .. } => Ok(value.max(0.0)),
        // CSS Color 4: 100% chroma in oklch equals 0.4 numeric.
        Token::Percentage { unit_value, .. } => Ok((unit_value * 0.4).max(0.0)),
        _ => Err(()),
    }
}

/// Parse the hue component and return it in radians.
fn parse_oklch_hue(parser: &mut Parser) -> Result<f32, ()> {
    match parser.next().map_err(|_| ())? {
        // Bare number is degrees per CSS Color 4.
        Token::Number { value, .. } => Ok(value.to_radians()),
        Token::Dimension { value, unit, .. } => match unit.as_ref() {
            "deg" => Ok(value.to_radians()),
            "rad" => Ok(*value),
            "grad" => Ok(value * std::f32::consts::PI / 200.0),
            "turn" => Ok(value * std::f32::consts::TAU),
            _ => Err(()),
        },
        _ => Err(()),
    }
}

/// Alpha as a `0.0..=1.0` float. Accepts a number or a percentage.
fn parse_alpha_unit(parser: &mut Parser) -> Result<f32, ()> {
    match parser.next().map_err(|_| ())? {
        Token::Number { value, .. } => Ok(value.clamp(0.0, 1.0)),
        Token::Percentage { unit_value, .. } => Ok(unit_value.clamp(0.0, 1.0)),
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

/// Parse `text-shadow`: `none` (empty list) or a comma-separated list of
/// layers. Mirrors [`parse_box_shadow_list`].
fn parse_text_shadow_list(parser: &mut Parser) -> Result<SmallVec<[ParsedTextShadow; 2]>, ()> {
    let mut layers: SmallVec<[ParsedTextShadow; 2]> = SmallVec::new();

    // `text-shadow: none` produces an empty list.
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
        layers.push(parse_single_text_shadow(parser)?);
        if parser.try_parse(cssparser::Parser::expect_comma).is_err() {
            break;
        }
    }

    Ok(layers)
}

/// Parse one `text-shadow` layer: `<color>? <offset-x> <offset-y> <blur>?`.
/// Per the CSS spec the color may appear before OR after the lengths; there is
/// no spread or inset. Omitted color stays `None` (resolved to the element
/// color at apply time).
fn parse_single_text_shadow(parser: &mut Parser) -> Result<ParsedTextShadow, ()> {
    // Color may lead.
    let mut color = parser.try_parse(|p| parse_color(p)).ok();

    // Required offsets.
    let offset_x = parse_px(parser)?;
    let offset_y = parse_px(parser)?;

    // Optional blur (defaults to 0).
    let blur_radius = parser.try_parse(|p| parse_px(p)).unwrap_or(0.0);

    // Color may trail instead of lead.
    if color.is_none() {
        color = parser.try_parse(|p| parse_color(p)).ok();
    }

    Ok(ParsedTextShadow { offset_x, offset_y, blur_radius, color })
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
        // A well-formed entry whose property name is not (yet) animatable is
        // skipped (`Ok(None)`) rather than failing the whole comma list, so the
        // remaining, supported properties still transition. A genuinely
        // malformed entry (bad duration/timing) still propagates as an error.
        if let Some(def) = parse_single_transition(parser)? {
            defs.push(def);
        }

        // Try to consume a comma for the next entry.
        if parser.try_parse(cssparser::Parser::expect_comma).is_err() {
            break;
        }
    }

    Ok(defs)
}

/// Parse a single transition entry: `<property> <duration> [<timing-function>] [<delay>]`
///
/// Returns `Ok(None)` when the entry is otherwise well-formed but names a
/// property the transition machinery does not animate (e.g. `left`). The
/// entry's tokens are still fully consumed so the parser cursor lands on the
/// following comma, letting the caller continue with the rest of the list
/// instead of dropping the entire `transition` declaration.
fn parse_single_transition(parser: &mut Parser) -> Result<Option<TransitionDef>, ()> {
    // Property name.
    let prop_name = parser.expect_ident().map_err(|_| ())?.to_string();
    let property = TransitionProperty::from_str(&prop_name);

    // Duration (required).
    let duration = parse_time_value(parser)?;

    // Timing function (optional, defaults to Ease).
    let timing_function = parser.try_parse(parse_timing_function).unwrap_or(TimingFunction::Ease);

    // Delay (optional, defaults to 0).
    let delay = parser.try_parse(parse_time_value).unwrap_or(Duration::ZERO);

    // Unknown / not-yet-animatable property: tokens consumed, entry skipped.
    let Some(property) = property else {
        return Ok(None);
    };

    Ok(Some(TransitionDef { property, duration, timing_function, delay }))
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
                        // Keyframe blocks have no custom-property scope of their
                        // own; any Deferred captured here is hinted at the base
                        // scope (key 0).
                        if let Ok(parsed) = parse_declaration(block, ScopeKey(0)) {
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
            } else if unit_str == "vh" || unit_str == "vw" {
                // Grid track sizes have no viewport context at parse time,
                // so vh/vw must fail rather than silently becoming px.
                // Mirrors the rejection in `parse_px`.
                Err(())
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
                    Token::Dimension { value, unit, .. } => {
                        if unit.as_ref() == "vh" || unit.as_ref() == "vw" {
                            // Viewport units cannot resolve at parse time.
                            Err(p.new_custom_error(()))
                        } else {
                            Ok(types::GridTrackSize {
                                min: types::GridMinTrackSize::Auto,
                                max: types::GridMaxTrackSize::FitContent(value),
                            })
                        }
                    }
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
            let unit_str = unit.as_ref();
            if unit_str == "%" {
                Ok(types::GridMinTrackSize::Percent(*value))
            } else if unit_str == "vh" || unit_str == "vw" {
                Err(())
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
            } else if unit_str == "vh" || unit_str == "vw" {
                Err(())
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
        // Padding writes both the resolved f32 mirror (paint/hit-test/transition)
        // and the unit-preserving source (layout, via to_taffy_style).
        StyleDeclaration::Padding(v) => {
            style.padding = *v;
            style.padding_src = EdgesDim::from_px(*v);
        }
        StyleDeclaration::PaddingTop(v) => {
            style.padding.top = *v;
            style.padding_src.top = Dimension::Px(*v);
        }
        StyleDeclaration::PaddingRight(v) => {
            style.padding.right = *v;
            style.padding_src.right = Dimension::Px(*v);
        }
        StyleDeclaration::PaddingBottom(v) => {
            style.padding.bottom = *v;
            style.padding_src.bottom = Dimension::Px(*v);
        }
        StyleDeclaration::PaddingLeft(v) => {
            style.padding.left = *v;
            style.padding_src.left = Dimension::Px(*v);
        }
        StyleDeclaration::PaddingDim(sides) => {
            // Non-px units resolve to 0 in the f32 mirror (paint has no viewport);
            // to_taffy_style resolves the real value from padding_src.
            let px_or_zero = |d: Dimension| match d {
                Dimension::Px(v) => v,
                _ => 0.0,
            };
            if let Some(d) = sides[0] {
                style.padding.top = px_or_zero(d);
                style.padding_src.top = d;
            }
            if let Some(d) = sides[1] {
                style.padding.right = px_or_zero(d);
                style.padding_src.right = d;
            }
            if let Some(d) = sides[2] {
                style.padding.bottom = px_or_zero(d);
                style.padding_src.bottom = d;
            }
            if let Some(d) = sides[3] {
                style.padding.left = px_or_zero(d);
                style.padding_src.left = d;
            }
        }
        StyleDeclaration::Margin(v) => {
            style.margin = *v;
            style.margin_auto = EdgeAutoFlags::NONE;
        }
        StyleDeclaration::MarginWithAuto(v, auto) => {
            style.margin = *v;
            style.margin_auto = *auto;
        }
        StyleDeclaration::MarginTop(v) => {
            style.margin.top = *v;
            style.margin_auto.top = false;
        }
        StyleDeclaration::MarginRight(v) => {
            style.margin.right = *v;
            style.margin_auto.right = false;
        }
        StyleDeclaration::MarginBottom(v) => {
            style.margin.bottom = *v;
            style.margin_auto.bottom = false;
        }
        StyleDeclaration::MarginLeft(v) => {
            style.margin.left = *v;
            style.margin_auto.left = false;
        }
        StyleDeclaration::MarginTopAuto => {
            style.margin.top = 0.0;
            style.margin_auto.top = true;
        }
        StyleDeclaration::MarginRightAuto => {
            style.margin.right = 0.0;
            style.margin_auto.right = true;
        }
        StyleDeclaration::MarginBottomAuto => {
            style.margin.bottom = 0.0;
            style.margin_auto.bottom = true;
        }
        StyleDeclaration::MarginLeftAuto => {
            style.margin.left = 0.0;
            style.margin_auto.left = true;
        }
        StyleDeclaration::Gap(v) => {
            style.row_gap = *v;
            style.column_gap = *v;
        }
        StyleDeclaration::RowGap(v) => style.row_gap = *v,
        StyleDeclaration::ColumnGap(v) => style.column_gap = *v,
        StyleDeclaration::OverflowX(v) => style.overflow_x = *v,
        StyleDeclaration::OverflowY(v) => style.overflow_y = *v,
        StyleDeclaration::Resize(v) => style.resize = *v,
        StyleDeclaration::BoxSizing(v) => style.box_sizing = *v,
        StyleDeclaration::AspectRatio(v) => style.aspect_ratio = *v,
        StyleDeclaration::ObjectFit(v) => style.object_fit = *v,
        StyleDeclaration::ObjectPosition(v) => style.object_position = *v,
        StyleDeclaration::Background(v) => style.background = v.clone(),
        StyleDeclaration::BorderColor(v) => style.border_color = *v,
        StyleDeclaration::BorderWidth(v) => style.border_width = *v,
        StyleDeclaration::BorderSideWidth(side, v) => match side {
            BorderSide::Top => style.border_width.top = *v,
            BorderSide::Right => style.border_width.right = *v,
            BorderSide::Bottom => style.border_width.bottom = *v,
            BorderSide::Left => style.border_width.left = *v,
        },
        // Single stored border_color; correct because every consuming rule
        // gives width to exactly one side (see the BorderSideColor doc).
        StyleDeclaration::BorderSideColor(_side, v) => style.border_color = *v,
        StyleDeclaration::BorderRadius(v) => {
            style.border_radius = *v;
            style.border_radius_src = CornersDim::from_px(*v);
        }
        StyleDeclaration::BorderRadiusDim(v) => {
            // Percent corners resolve to 0 in the f32 mirror (paint has no box);
            // the renderer resolves the real value from border_radius_src. Px
            // corners still write through so the mirror stays usable for
            // transition lerp / DPI scaling of pure-px radii.
            let px_or_zero = |c: LengthOrPercent| match c {
                LengthOrPercent::Px(px) => px,
                LengthOrPercent::Percent(_) => 0.0,
            };
            style.border_radius = Corners {
                top_left: px_or_zero(v.top_left),
                top_right: px_or_zero(v.top_right),
                bottom_right: px_or_zero(v.bottom_right),
                bottom_left: px_or_zero(v.bottom_left),
            };
            style.border_radius_src = *v;
        }
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
        StyleDeclaration::TextShadowList(v) => {
            let mut out: SmallVec<[types::TextShadow; 2]> = SmallVec::with_capacity(v.len());
            for layer in v.iter() {
                // Omitted color defaults to the element's `color` (CSS
                // `currentColor`). Blur is clamped non-negative and bounded so a
                // pathological radius can't blow up the stacked-tap glow.
                let color = layer.color.unwrap_or(style.color);
                out.push(types::TextShadow {
                    offset_x: layer.offset_x,
                    offset_y: layer.offset_y,
                    blur_radius: layer.blur_radius.clamp(0.0, 64.0),
                    color,
                });
            }
            style.text_shadow = out;
        }
        StyleDeclaration::BackdropFilter(v) => style.backdrop_filter = Some(v.clone()),
        StyleDeclaration::Color(v) => style.color = *v,
        StyleDeclaration::FontSize(v) => {
            style.font_size = *v;
            style.font_size_explicit = true;
        }
        StyleDeclaration::FontScale(v) => style.font_size_scale = v.clamp(0.25, 4.0),
        StyleDeclaration::FontWeight(v) => style.font_weight = *v,
        StyleDeclaration::FontStyle(v) => style.font_style = *v,
        StyleDeclaration::FontFamily(v) => style.font_family = v.clone(),
        StyleDeclaration::LineHeight(v) => style.line_height = *v,
        StyleDeclaration::LetterSpacing(v) => style.letter_spacing = *v,
        StyleDeclaration::TextAlign(v) => style.text_align = *v,
        StyleDeclaration::TextTransform(v) => style.text_transform = *v,
        StyleDeclaration::TextDecoration(v) => style.text_decoration = *v,
        StyleDeclaration::TextDecorationColor(v) => style.text_decoration_color = Some(*v),
        StyleDeclaration::WhiteSpace(v) => style.white_space = *v,
        StyleDeclaration::TextOverflow(v) => style.text_overflow = *v,
        StyleDeclaration::Cursor(v) => style.cursor = *v,
        StyleDeclaration::Visibility(v) => style.visibility = *v,
        StyleDeclaration::PointerEvents(v) => style.pointer_events = *v,
        StyleDeclaration::UserSelect(v) => style.user_select = *v,
        StyleDeclaration::AppRegion(v) => style.app_region = *v,
        StyleDeclaration::Position(v) => style.position = *v,
        StyleDeclaration::Top(v) => style.top = Some(*v),
        StyleDeclaration::Right(v) => style.right = Some(*v),
        StyleDeclaration::Bottom(v) => style.bottom = Some(*v),
        StyleDeclaration::Left(v) => style.left = Some(*v),
        StyleDeclaration::ZIndex(v) => style.z_index = *v,
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

        // CSS `transform`. Replaces any prior value on the same element so
        // later declarations win (standard cascade rule).
        StyleDeclaration::Transform(v) => style.transform = *v,

        // CSS `mask-image: linear-gradient(...)`. Only the linear gradient
        // form is supported; see `parse_mask_image`.
        StyleDeclaration::MaskImage(g) => style.mask_image = Some(g.clone()),

        // A `Deferred` carrier cannot be applied here: resolving its `var()`
        // needs the element's token scope env and any drop must be routed to a
        // `dropped` sink, neither of which this signature carries. The cascade
        // intercepts `Deferred` BEFORE calling `apply_declaration` and routes it
        // through `apply_deferred_against_env` (which has that context), so this
        // arm is a no-op safety net only — reached if a `Deferred` is ever fed to
        // the bare apply path (e.g. via `style_overrides`), in which case
        // silently skipping it is safer than mis-typing it.
        StyleDeclaration::Deferred { .. } => {}
    }
}

/// Apply a [`StyleDeclaration::Deferred`] carrier: resolve its `var()` value,
/// re-parse the resulting concrete text into a real typed declaration, and apply
/// that. This is the apply path the bare [`apply_declaration`] cannot provide,
/// because it needs the custom-property table (`props`) to resolve `var()` and
/// the stylesheet's `dropped` sink to record values the re-parse rejects.
///
/// Resolution this stage is `:root`-only — i.e. identical to the current global
/// behavior — so `props` is the flat `:root` custom-property map; `scope_hint`
/// is carried for the later stage that resolves per matching scope and is unused
/// here beyond being echoed into any `DroppedDeclaration`.
///
/// If the resolved text still cannot be typed (an engine gap, or an unresolved
/// `var()` with no fallback), the declaration is routed into `dropped` rather
/// than silently swallowed, mirroring the eager parser's drop path.
pub fn apply_deferred_declaration(
    style: &mut ComputedStyle,
    property: &str,
    raw_value: &str,
    scope_hint: ScopeKey,
    props: &HashMap<String, String>,
    dropped: &mut Vec<DroppedDeclaration>,
) {
    // Resolve var() against :root (current global behavior). `flatten_token_value`
    // iterates with a cycle guard, leaving any unresolvable `var(` in place.
    let resolved = flatten_token_value(raw_value, props);

    // Re-parse the concrete `property: value` text as a one-declaration sheet.
    let decl_text = format!("{property}: {resolved}");
    let mut input = ParserInput::new(&decl_text);
    let mut parser = Parser::new(&mut input);
    // `scope_hint` does not change the re-parse itself: the resolved text has no
    // `var(` left (or an unresolvable one that will route to `dropped`), so the
    // typed match runs. Pass the base scope to satisfy the signature.
    match parse_declaration(&mut parser, ScopeKey(0)) {
        Ok(decls) if !decls.iter().any(|d| matches!(d, StyleDeclaration::Deferred { .. })) => {
            for decl in &decls {
                apply_declaration(style, decl);
            }
        }
        // Either the re-parse failed outright, or it round-tripped back to a
        // `Deferred` (an unresolved `var(` survived). Route to `dropped` so the
        // gap is visible instead of being swallowed.
        _ => {
            record_dropped_declaration(
                dropped,
                &deferred_scope_label(scope_hint),
                &format!("{property}: {resolved}"),
            );
        }
    }
}

/// Best-effort diagnostic label for a dropped `Deferred` value. The carrier only
/// stores a [`ScopeKey`], not selector text, so the drop records the key. This
/// keeps the routed `DroppedDeclaration` non-empty and traceable without
/// plumbing the full scope table into the apply path.
fn deferred_scope_label(scope: ScopeKey) -> String {
    format!("<deferred scope {}>", scope.0)
}

/// The ordered token-resolution environment for ONE element. Holds borrowed
/// handles to the pre-flattened `--name -> value` maps of the scopes that are
/// active for the element, ordered HIGHEST SPECIFICITY FIRST (a widget self
/// scope like `.theme-chip.dracula`, then the active root theme `.app.theme-*`,
/// then the `:root` base). Token lookups walk the list in order and take the
/// first hit, so an override in a more-specific scope wins — exactly the cascade
/// the browser would apply to a custom property.
///
/// Almost always this is `[:root]` or `[:root, theme]`; the self-scope slot only
/// appears for the handful of widget elements that carry their own token class.
///
/// The maps hold RAW token values (token->token references are NOT pre-flattened
/// by `collect_token_scopes`), so a lookup can return a value that is itself a
/// `var()` reference or carries a `var()` fallback. `flatten_token_value_env`
/// keeps unwinding those references against this same ordered env each pass —
/// MULTI-LEVEL resolution — so a token redefined on the active theme propagates
/// to every consumer, including those that reach it through a base-scope alias.
#[derive(Clone, Copy, Default)]
pub struct ScopeEnv<'a> {
    /// Up to three borrowed scope maps, highest-specificity-first. A fixed-size
    /// array (not a `Vec`) so building an env is allocation-free per element.
    maps: [Option<&'a HashMap<String, String>>; 3],
}

impl<'a> ScopeEnv<'a> {
    /// Build the env for an element from its active scopes, highest-specificity
    /// first. `self_scope` is the element's own widget token scope (if any);
    /// `root_scope` is the active `.app.theme-*` root scope (if any); `base` is
    /// the `:root` base map. `None` slots are skipped on lookup.
    pub fn new(
        self_scope: Option<&'a HashMap<String, String>>,
        root_scope: Option<&'a HashMap<String, String>>,
        base: Option<&'a HashMap<String, String>>,
    ) -> Self {
        ScopeEnv { maps: [self_scope, root_scope, base] }
    }

    /// Look up a custom-property name across the env, highest-specificity first.
    fn get(&self, name: &str) -> Option<&'a String> {
        self.maps.iter().flatten().find_map(|m| m.get(name))
    }

    /// Fully resolve `raw`'s `var()` references against this env, unwinding
    /// token->token references MULTI-LEVEL against the same ordered env each pass
    /// (highest-specificity scope first, then `:root`, then the var() fallback),
    /// with a cycle guard. Any `var()` that cannot resolve (no token + no
    /// fallback) is left as a literal `var(...)` in the result. This is the
    /// public entry point onto the use-time resolver the cascade applies to a
    /// `Deferred` carrier; exposed for tests and callers that need the resolved
    /// token text directly.
    pub fn resolve_value(raw: &str, env: &ScopeEnv) -> String {
        flatten_token_value_env(raw, env)
    }
}

/// Resolve `var(--name[, fallback])` references in `raw` against the ordered
/// `env`, iterating so a fallback that is itself a `var()` keeps unwinding. The
/// name/fallback split reuses [`find_top_level_comma`]; the substitution reuses
/// the same single-pass shape as [`resolve_var_once`] but resolves names through
/// the env's specificity-ordered lookup instead of a single flat map. Any name
/// the env cannot resolve (and that has no fallback) is left as a literal
/// `var(...)` so the re-parse fails and the value routes to `dropped`.
fn flatten_token_value_env(raw: &str, env: &ScopeEnv) -> String {
    let mut value = raw.to_string();
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    for _ in 0..32 {
        if !value.contains("var(") {
            break;
        }
        // Cycle / fixed-point guard: if the exact text repeats, stop.
        if !visited.insert(value.clone()) {
            break;
        }
        match resolve_var_once_env(&value, env) {
            Some(next) => value = next,
            None => break,
        }
    }
    value
}

/// Single env-aware pass of `var()` substitution. Mirrors [`resolve_var_once`]
/// but resolves each name through [`ScopeEnv::get`] (specificity-ordered) rather
/// than a single flat map. Returns `None` when nothing changed.
fn resolve_var_once_env(css: &str, env: &ScopeEnv) -> Option<String> {
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

            if let Some(value) = env.get(var_name) {
                result.push_str(value);
                changed = true;
            } else if let Some(fb) = fallback {
                result.push_str(fb);
                changed = true;
            } else {
                // Unresolvable, no fallback: keep the literal var() so the
                // re-parse fails and the value routes to `dropped`.
                result.push_str(prefix);
                result.push_str(var_content);
                result.push(')');
            }

            remaining = rest;
        } else {
            result.push_str(prefix);
            remaining = after_var;
        }
    }

    result.push_str(remaining);
    changed.then_some(result)
}

/// Apply a [`StyleDeclaration::Deferred`] carrier against an element's ordered
/// token [`ScopeEnv`]. This is the cascade's live apply path (Stage 3): resolve
/// `var()` against the element's active scopes (self > theme > `:root`), re-parse
/// the concrete text into a typed declaration, and apply it. A re-parse failure
/// (engine gap, or an unresolved `var()` with no fallback) routes into `dropped`
/// rather than being silently swallowed.
///
/// `scope_hint` is the [`ScopeKey`] of the block the declaration was authored in;
/// it is a backstop label for any drop and a tiebreaker, not the primary resolve
/// source — the env already encodes the element's active scopes in specificity
/// order.
pub fn apply_deferred_against_env(
    style: &mut ComputedStyle,
    property: &str,
    raw_value: &str,
    scope_hint: ScopeKey,
    env: &ScopeEnv,
    dropped: &mut Vec<DroppedDeclaration>,
) {
    match resolve_deferred_to_decls(property, raw_value, env) {
        Ok(decls) => {
            for decl in &decls {
                apply_declaration(style, decl);
            }
        }
        Err(resolved) => {
            record_dropped_declaration(
                dropped,
                &deferred_scope_label(scope_hint),
                &format!("{property}: {resolved}"),
            );
        }
    }
}

/// Resolve a deferred `property: raw_value` against `env` and re-parse the
/// concrete text into typed declarations. `Ok(decls)` is the typed result (empty
/// only if the value typed to nothing); `Err(resolved_text)` means the resolved
/// text could not be typed (engine gap, or an unresolved `var()` with no
/// fallback) — the caller routes it to `dropped`. Factored out of
/// [`apply_deferred_against_env`] so the cascade can MEMOIZE this (the expensive
/// part: a string substitution plus a cssparser re-parse) per
/// `(active_root_scope, property, raw_value)` when no self scope is involved.
fn resolve_deferred_to_decls(
    property: &str,
    raw_value: &str,
    env: &ScopeEnv,
) -> Result<SmallVec<[StyleDeclaration; 2]>, String> {
    let resolved = flatten_token_value_env(raw_value, env);
    let decl_text = format!("{property}: {resolved}");
    let mut input = ParserInput::new(&decl_text);
    let mut parser = Parser::new(&mut input);
    match parse_declaration(&mut parser, ScopeKey(0)) {
        Ok(decls) if !decls.iter().any(|d| matches!(d, StyleDeclaration::Deferred { .. })) => {
            Ok(decls)
        }
        _ => Err(resolved),
    }
}

thread_local! {
    /// Per-thread memo for the cascade's deferred-declaration apply. Keyed by
    /// `(stylesheet identity, active_root_scope, property, raw_value)` and only
    /// used when the element has NO self widget scope (the overwhelmingly common
    /// case, where the env is purely `[active_root_scope, :root]` — both pass-wide
    /// globals, so the resolved typed declarations are identical for every
    /// element that matches the same rule). The `stylesheet identity` (a pointer
    /// taken at the call site) changes on re-parse / hot reload, so a stale entry
    /// can never be served for a different stylesheet; when it changes we clear
    /// the memo so it cannot grow unbounded across reloads.
    static DEFERRED_MEMO: std::cell::RefCell<DeferredMemo> =
        std::cell::RefCell::new(DeferredMemo::default());
}

#[derive(Default)]
struct DeferredMemo {
    stylesheet_id: u64,
    /// `(active_root_scope, property, raw_value) -> Ok(typed decls) | Err(resolved)`.
    /// The same `Result` shape `resolve_deferred_to_decls` returns, so the apply
    /// path is identical on a hit.
    map: HashMap<(u32, Box<str>, Box<str>), Result<SmallVec<[StyleDeclaration; 2]>, String>>,
}

/// Apply a deferred declaration through the per-pass memo. `stylesheet_id` is a
/// stable-within-a-parse identity (the stylesheet's process-unique `parse_id`);
/// `active_root_scope` is the pass-wide theme key.
/// `has_self_scope` is true when the element carries its own widget token scope —
/// in that case the resolution depends on per-element state and the memo is
/// BYPASSED (resolved fresh) to stay correct. The common path (no self scope)
/// hits the memo, so steady-state cost is ~ a `HashMap` lookup plus the same
/// apply the concrete-decl path already does.
#[allow(clippy::too_many_arguments)]
pub fn apply_deferred_against_env_memoized(
    style: &mut ComputedStyle,
    property: &str,
    raw_value: &str,
    scope_hint: ScopeKey,
    env: &ScopeEnv,
    dropped: &mut Vec<DroppedDeclaration>,
    stylesheet_id: u64,
    active_root_scope: Option<ScopeKey>,
    has_self_scope: bool,
) {
    if has_self_scope {
        // Self scope is per-element; do not memoize against the pass-wide key.
        apply_deferred_against_env(style, property, raw_value, scope_hint, env, dropped);
        return;
    }

    let scope_key = active_root_scope.map(|k| k.0).unwrap_or(u32::MAX);
    DEFERRED_MEMO.with(|cell| {
        let mut memo = cell.borrow_mut();
        if memo.stylesheet_id != stylesheet_id {
            memo.stylesheet_id = stylesheet_id;
            memo.map.clear();
        }
        let key = (scope_key, Box::from(property), Box::from(raw_value));
        let entry = memo
            .map
            .entry(key)
            .or_insert_with(|| resolve_deferred_to_decls(property, raw_value, env));
        match entry {
            Ok(decls) => {
                for decl in decls.iter() {
                    apply_declaration(style, decl);
                }
            }
            Err(resolved) => {
                record_dropped_declaration(
                    dropped,
                    &deferred_scope_label(scope_hint),
                    &format!("{property}: {resolved}"),
                );
            }
        }
    });
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

    /// Apply a rule's declarations to a fresh [`ComputedStyle`] the way the
    /// cascade does for a `:root`-base element: concrete declarations apply
    /// directly, and `Deferred` carriers resolve against the stylesheet's BASE
    /// token scope (`:root`/`*`) before re-parsing. This is the post-Stage-3
    /// stand-in for the old "var() resolved at parse time" behavior the unit
    /// tests below exercise: the resolution simply moved from parse time into
    /// this base-scope env apply.
    fn apply_rule_with_base_env(
        sheet: &CompiledStylesheet,
        decls: &[StyleDeclaration],
    ) -> ComputedStyle {
        let env = ScopeEnv::new(None, None, sheet.token_scopes.base_vars());
        let mut style = ComputedStyle::default();
        let mut dropped = Vec::new();
        for decl in decls {
            match decl {
                StyleDeclaration::Deferred { property, raw_value, scope_hint } => {
                    apply_deferred_against_env(
                        &mut style,
                        property,
                        raw_value,
                        *scope_hint,
                        &env,
                        &mut dropped,
                    );
                }
                _ => apply_declaration(&mut style, decl),
            }
        }
        style
    }

    /// Find the first non-empty rule whose selector carries `class`, returning
    /// its declarations. Panics if no such rule exists.
    fn rule_decls_for_class<'a>(
        sheet: &'a CompiledStylesheet,
        class: &str,
    ) -> &'a [StyleDeclaration] {
        sheet
            .rules
            .iter()
            .find(|rule| {
                !rule.declarations.is_empty()
                    && rule.selector.parts.iter().any(|(parts, _)| {
                        parts
                            .iter()
                            .any(|part| matches!(part, SelectorPart::Class(c) if c == class))
                    })
            })
            .map(|r| r.declarations.as_slice())
            .unwrap_or_else(|| panic!("expected a .{class} rule with declarations"))
    }

    #[test]
    fn test_border_radius_percent_parses_to_corners_dim() {
        // `50%` must survive parsing as a unit-preserving CornersDim (the f32
        // fast path would silently drop it, the original engine gap).
        let decls = parse_decls(".x { border-radius: 50%; }");
        let corners = decls
            .iter()
            .find_map(|d| match d {
                StyleDeclaration::BorderRadiusDim(c) => Some(*c),
                _ => None,
            })
            .expect("border-radius: 50% should parse to BorderRadiusDim");
        // 50% becomes the unit fraction 0.5 on every corner.
        let half = LengthOrPercent::Percent(0.5);
        assert_eq!(
            corners,
            CornersDim { top_left: half, top_right: half, bottom_right: half, bottom_left: half }
        );
        // Resolving against a 40x40 box yields 20px on every corner.
        assert_eq!(corners.resolve(40.0_f32.min(40.0)), [20.0, 20.0, 20.0, 20.0]);
    }

    #[test]
    fn test_border_radius_percent_apply_zeroes_f32_mirror() {
        // The f32 `border_radius` mirror has no box at apply time, so percent
        // corners resolve to 0 there; the real value lives in border_radius_src.
        let decls = parse_decls(".x { border-radius: 50%; }");
        let mut style = ComputedStyle::default();
        for decl in &decls {
            apply_declaration(&mut style, decl);
        }
        assert_eq!(style.border_radius, Corners::ZERO);
        assert_eq!(
            style.border_radius_src,
            CornersDim {
                top_left: LengthOrPercent::Percent(0.5),
                top_right: LengthOrPercent::Percent(0.5),
                bottom_right: LengthOrPercent::Percent(0.5),
                bottom_left: LengthOrPercent::Percent(0.5),
            }
        );
        // Paint-time resolution against a 40x40 box is circular: 20px corners.
        assert_eq!(style.border_radius_src.resolve(40.0), [20.0, 20.0, 20.0, 20.0]);
    }

    #[test]
    fn test_overflow_per_axis_longhands_set_independent_fields() {
        // The axes genuinely differ in the real stylesheet, so each longhand
        // must write only its own field. `auto` maps to `Scroll`.
        let decls = parse_decls(".x { overflow-x: hidden; overflow-y: auto; }");
        assert_eq!(
            decls,
            vec![
                StyleDeclaration::OverflowX(Overflow::Hidden),
                StyleDeclaration::OverflowY(Overflow::Scroll),
            ]
        );

        let mut style = ComputedStyle::default();
        for decl in &decls {
            apply_declaration(&mut style, decl);
        }
        assert_eq!(style.overflow_x, Overflow::Hidden);
        assert_eq!(style.overflow_y, Overflow::Scroll);
    }

    #[test]
    fn test_overflow_shorthand_sets_both_axes() {
        // The `overflow` shorthand expands to both per-axis longhands.
        let decls = parse_decls(".x { overflow: hidden; }");
        assert_eq!(
            decls,
            vec![
                StyleDeclaration::OverflowX(Overflow::Hidden),
                StyleDeclaration::OverflowY(Overflow::Hidden),
            ]
        );

        let mut style = ComputedStyle::default();
        for decl in &decls {
            apply_declaration(&mut style, decl);
        }
        assert_eq!(style.overflow_x, Overflow::Hidden);
        assert_eq!(style.overflow_y, Overflow::Hidden);
    }

    #[test]
    fn test_border_radius_px_takes_f32_fast_path() {
        // Pure-px radii keep the f32 `BorderRadius` variant so transitions and
        // DPI scaling behave exactly as before.
        let decls = parse_decls(".x { border-radius: 6px; }");
        let radius = decls.iter().find_map(|d| match d {
            StyleDeclaration::BorderRadius(c) => Some(*c),
            _ => None,
        });
        assert_eq!(radius, Some(Corners::all(6.0)));
        // It must NOT also emit the unit-preserving variant.
        assert!(!decls.iter().any(|d| matches!(d, StyleDeclaration::BorderRadiusDim(_))));

        // Apply keeps both the f32 mirror and the src in sync (all-px).
        let mut style = ComputedStyle::default();
        for decl in &decls {
            apply_declaration(&mut style, decl);
        }
        assert_eq!(style.border_radius, Corners::all(6.0));
        assert_eq!(style.border_radius_src, CornersDim::from_px(Corners::all(6.0)));
    }

    #[test]
    fn test_font_shorthand_expands_to_supported_longhands() {
        let decls = parse_decls(r#".x { font: 600 13px/1.4 "JetBrains Mono", monospace; }"#);
        let mut style = ComputedStyle::default();
        for decl in &decls {
            apply_declaration(&mut style, decl);
        }

        assert_eq!(style.font_weight, FontWeight::W(600));
        assert!((style.font_size - 13.0).abs() < 0.01);
        assert!((style.line_height - 1.4).abs() < 0.01);
        assert_eq!(style.font_family, "JetBrains Mono, monospace");
    }

    #[test]
    fn test_font_style_parses_and_applies() {
        let decls = parse_decls(".x { font-style: italic; }");
        assert_eq!(decls.as_slice(), [StyleDeclaration::FontStyle(FontStyle::Italic)]);

        let mut style = ComputedStyle::default();
        for decl in &decls {
            apply_declaration(&mut style, decl);
        }
        assert_eq!(style.font_style, FontStyle::Italic);
    }

    #[test]
    fn test_font_style_oblique_and_normal() {
        let oblique = parse_decls(".x { font-style: oblique; }");
        assert_eq!(oblique.as_slice(), [StyleDeclaration::FontStyle(FontStyle::Oblique)]);

        let normal = parse_decls(".x { font-style: normal; }");
        assert_eq!(normal.as_slice(), [StyleDeclaration::FontStyle(FontStyle::Normal)]);
    }

    #[test]
    fn test_font_style_inherits() {
        let mut parent = ComputedStyle::default();
        parent.font_style = FontStyle::Italic;
        let mut child = ComputedStyle::default();
        child.inherit_from(&parent);
        assert_eq!(child.font_style, FontStyle::Italic);
    }

    #[test]
    fn test_font_family_preserves_fallback_list() {
        let decls = parse_decls(
            r#".x { font-family: "JetBrains Mono", "Berkeley Mono", "SF Mono", Menlo, Consolas, monospace; }"#,
        );
        let mut style = ComputedStyle::default();
        for decl in &decls {
            apply_declaration(&mut style, decl);
        }

        assert_eq!(
            style.font_family,
            "JetBrains Mono, Berkeley Mono, SF Mono, Menlo, Consolas, monospace"
        );
    }

    #[test]
    fn test_text_transform_cascades() {
        let decls = parse_decls(".x { text-transform: uppercase; }");
        let mut style = ComputedStyle::default();
        for decl in &decls {
            apply_declaration(&mut style, decl);
        }

        assert_eq!(style.text_transform, TextTransform::Uppercase);
    }

    #[test]
    fn test_font_shorthand_var_reference_resolves_design_token() {
        let sheet = CompiledStylesheet::parse(
            r#"
            .x { font: var(--type-body); }
            :root {
                --t-md: 12px;
                --font-mono: "JetBrains Mono", Consolas, monospace;
                --type-body: 400 var(--t-md)/1.55 var(--font-mono);
            }
            "#,
        );
        // `--type-body` itself nests `var(--t-md)`/`var(--font-mono)`; the base
        // scope pre-flattens those, and `.x { font: var(--type-body) }` defers
        // then resolves against the base env.
        let decls = rule_decls_for_class(&sheet, "x").to_vec();
        let style = apply_rule_with_base_env(&sheet, &decls);

        assert_eq!(style.font_weight, FontWeight::W(400));
        assert!((style.font_size - 12.0).abs() < 0.01);
        assert!((style.line_height - 1.55).abs() < 0.01);
        assert_eq!(style.font_family, "JetBrains Mono, Consolas, monospace");
    }

    #[test]
    fn test_font_shorthand_resolves_settings_page_type_variable() {
        let sheet = CompiledStylesheet::parse(
            r#"
            :root {
                --font-mono: "JetBrains Mono", "Berkeley Mono", monospace;
                --t-sm: 11px;
                --type-label: 500 var(--t-sm)/1.4 var(--font-mono);
            }
            .set-page-nav-item { font: var(--type-label); }
            "#,
        );
        let decls = rule_decls_for_class(&sheet, "set-page-nav-item").to_vec();
        let style = apply_rule_with_base_env(&sheet, &decls);

        assert_eq!(style.font_weight, FontWeight::W(500));
        assert!((style.font_size - 11.0).abs() < 0.01);
        assert!((style.line_height - 1.4).abs() < 0.01);
        assert_eq!(style.font_family, "JetBrains Mono, Berkeley Mono, monospace");
    }

    #[test]
    fn test_invalid_font_shorthand_does_not_swallow_next_declaration() {
        let decls = parse_decls(".x { font: 12px; color: #ff0000; }");
        assert!(!decls.iter().any(|d| matches!(d, StyleDeclaration::FontSize(_))));
        assert!(decls
            .iter()
            .any(|d| matches!(d, StyleDeclaration::Color(Color { r: 255, g: 0, b: 0, a: 255 }))));
    }

    #[test]
    fn test_border_shorthand_sets_width_and_color() {
        let decls = parse_decls(".x { border: 1px solid #112233; }");
        let mut style = ComputedStyle::default();
        for decl in &decls {
            apply_declaration(&mut style, decl);
        }

        assert_eq!(style.border_width, Edges::all(1.0));
        assert_eq!(style.border_color, Color::rgb(0x11, 0x22, 0x33));
    }

    #[test]
    fn test_border_side_shorthand_sets_one_side_and_color() {
        let decls = parse_decls(".x { border-bottom: 2px dashed #c9553a; }");
        let mut style = ComputedStyle::default();
        for decl in &decls {
            apply_declaration(&mut style, decl);
        }

        assert_eq!(style.border_width.bottom, 2.0);
        assert_eq!(style.border_width.top, 0.0);
        assert_eq!(style.border_width.right, 0.0);
        assert_eq!(style.border_width.left, 0.0);
        assert_eq!(style.border_color, Color::rgb(0xc9, 0x55, 0x3a));
    }

    #[test]
    fn test_border_side_color_longhand_sets_color_and_preserves_width() {
        // `border-bottom-color` was previously unrecognized and silently
        // dropped. It must set the (single) border_color and leave the width
        // from the base `border-bottom` declaration intact.
        let decls =
            parse_decls(".x { border-bottom: 1px solid #111111; border-bottom-color: #c9553a; }");
        let mut style = ComputedStyle::default();
        for decl in &decls {
            apply_declaration(&mut style, decl);
        }

        assert_eq!(style.border_width.bottom, 1.0);
        assert_eq!(style.border_color, Color::rgb(0xc9, 0x55, 0x3a));
    }

    #[test]
    fn test_border_side_color_transparent_keeps_width() {
        // `.setting-row:hover { border-bottom-color: transparent }` must zero
        // only the color (hiding the divider) while keeping the 1px width.
        let decls = parse_decls(
            ".x { border-bottom: 1px solid #111111; border-bottom-color: transparent; }",
        );
        let mut style = ComputedStyle::default();
        for decl in &decls {
            apply_declaration(&mut style, decl);
        }

        assert_eq!(style.border_width.bottom, 1.0);
        assert_eq!(style.border_color, Color::TRANSPARENT);
    }

    #[test]
    fn test_all_four_border_side_color_arms_parse() {
        for side in ["top", "right", "bottom", "left"] {
            let css = format!(".x {{ border-{side}-color: #00ff00; }}");
            let decls = parse_decls(&css);
            assert!(
                decls.iter().any(|d| matches!(d, StyleDeclaration::BorderSideColor(_, _))),
                "border-{side}-color should not be dropped"
            );
        }
    }

    #[test]
    fn test_padding_px_keeps_f32_fast_path_and_mirrors_src() {
        let decls = parse_decls(".x { padding: 16px; }");
        assert!(decls.iter().any(|d| matches!(d, StyleDeclaration::Padding(_))));
        let mut style = ComputedStyle::default();
        for d in &decls {
            apply_declaration(&mut style, d);
        }
        assert_eq!(style.padding, Edges::all(16.0));
        assert_eq!(style.padding_src, EdgesDim::from_px(Edges::all(16.0)));
    }

    #[test]
    fn test_padding_unitless_zero_uses_px_fast_path() {
        // `padding: 0` (unitless) must still parse via the f32 fast path.
        let decls = parse_decls(".x { padding: 0; }");
        assert!(decls.iter().any(|d| matches!(d, StyleDeclaration::Padding(_))));
    }

    #[test]
    fn test_padding_top_vh_is_no_longer_dropped() {
        // Previously `padding-top: 12vh` routed through parse_px which rejects
        // vh, dropping the whole declaration. Now it parses to PaddingDim and
        // keeps the unit in padding_src (f32 mirror stays 0 for paint).
        let decls = parse_decls(".x { padding-top: 12vh; }");
        assert!(decls.iter().any(|d| matches!(d, StyleDeclaration::PaddingDim(_))));
        let mut style = ComputedStyle::default();
        for d in &decls {
            apply_declaration(&mut style, d);
        }
        assert_eq!(style.padding.top, 0.0);
        assert_eq!(style.padding_src.top, Dimension::Vh(12.0));
        // untouched sides stay at the default px(0)
        assert_eq!(style.padding_src.left, Dimension::Px(0.0));
    }

    #[test]
    fn test_padding_mixed_px_and_vh_shorthand_preserves_per_side_units() {
        let decls = parse_decls(".x { padding: 1px 2vh; }");
        let mut style = ComputedStyle::default();
        for d in &decls {
            apply_declaration(&mut style, d);
        }
        assert_eq!(style.padding_src.top, Dimension::Px(1.0));
        assert_eq!(style.padding_src.right, Dimension::Vh(2.0));
        assert_eq!(style.padding_src.bottom, Dimension::Px(1.0));
        assert_eq!(style.padding_src.left, Dimension::Vh(2.0));
    }

    #[test]
    fn test_strip_css_comments() {
        assert_eq!(strip_css_comments("a /* x */ b"), "a  b");
        assert_eq!(strip_css_comments("/* only */"), "");
        assert_eq!(strip_css_comments("no comment"), "no comment");
        // Unterminated comment drops the remainder, matching CSS tokenizing.
        assert_eq!(strip_css_comments("a /* unterminated"), "a ");
    }

    #[test]
    fn test_comment_with_colon_does_not_break_following_custom_property() {
        // A comment between :root declarations — especially one containing `:` —
        // must not break collection of the custom property after it. Regression
        // for the naive `;`-split in extract_custom_properties.
        let css = concat!(
            ":root {\n",
            "  --a: #111111;\n",
            "  /* note: this comment has a colon and precedes --accent */\n",
            "  --accent: #abcdef;\n",
            "}\n",
            ".x { color: var(--accent); }\n",
        );
        let sheet = CompiledStylesheet::parse(css);
        assert_eq!(sheet.custom_properties.get("--accent").map(String::as_str), Some("#abcdef"));
        // `--accent` must also reach the base TOKEN SCOPE (not just the flat
        // custom_properties map) so the cascade can resolve it: the comment with
        // a colon before it must not break that collection.
        assert_eq!(
            sheet.token_scopes.base_vars().and_then(|m| m.get("--accent")).map(String::as_str),
            Some("#abcdef"),
        );
        // `.x { color: var(--accent) }` defers, then resolves to #abcdef against
        // the base env — and nothing is dropped.
        let decls = rule_decls_for_class(&sheet, "x").to_vec();
        let style = apply_rule_with_base_env(&sheet, &decls);
        assert_eq!(style.color, Color::rgb(0xab, 0xcd, 0xef), "resolved color should apply to .x");
    }

    #[test]
    fn test_dropped_declarations_are_recorded() {
        // `mix-blend-mode` is a still-unsupported property (see the
        // stylesheet-coverage KNOWN_UNSUPPORTED inventory), so the parser drops
        // it. (`text-overflow` used to serve here but is now supported.)
        let sheet = CompiledStylesheet::parse(".x { mix-blend-mode: multiply; color: #ffffff; }");
        assert!(
            sheet.dropped.iter().any(|d| d.property == "mix-blend-mode" && d.value == "multiply"),
            "unrecognized property should be recorded: {:?}",
            sheet.dropped
        );
        // Valid declarations are not recorded as dropped.
        assert!(!sheet.dropped.iter().any(|d| d.property == "color"));
    }

    #[test]
    fn test_border_none_clears_width_without_swallowing_next_declaration() {
        let decls = parse_decls(".x { border: none; color: #ff0000; }");
        let mut style = ComputedStyle { border_width: Edges::all(1.0), ..Default::default() };
        for decl in &decls {
            apply_declaration(&mut style, decl);
        }

        assert_eq!(style.border_width, Edges::ZERO);
        assert_eq!(style.color, Color::rgb(255, 0, 0));
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
    fn test_transition_multi_property_with_transform_not_dropped() {
        // Regression: a transition list that names `transform` used to drop the
        // ENTIRE declaration because `transform` was absent from
        // `TransitionProperty::from_str`, erroring `parse_single_transition` and
        // discarding the whole comma list. The list must now survive and contain
        // both the background-color and the transform entry.
        let decls =
            parse_decls(".x { transition: background-color 0.2s ease, transform 0.2s ease; }");
        let defs = decls
            .iter()
            .find_map(|d| match d {
                StyleDeclaration::Transition(v) => Some(v),
                _ => None,
            })
            .expect("multi-property transition naming `transform` must not drop");

        assert_eq!(defs.len(), 2);

        // background-color maps onto the Background transition property.
        assert_eq!(defs[0].property, TransitionProperty::Background);
        assert!((defs[0].duration.as_secs_f32() - 0.2).abs() < 0.01);
        assert_eq!(defs[0].timing_function, TimingFunction::Ease);

        // transform is now a first-class animatable property (translateX).
        assert_eq!(defs[1].property, TransitionProperty::Transform);
        assert!((defs[1].duration.as_secs_f32() - 0.2).abs() < 0.01);
        assert_eq!(defs[1].timing_function, TimingFunction::Ease);
    }

    #[test]
    fn test_transition_skips_unanimatable_property_keeps_rest() {
        // `left` is not animatable, but it must NOT drop the whole declaration:
        // the well-formed `background` entry has to survive. Mirrors the real
        // stylesheet form `.toggle::after { transition: left ..., background ... }`.
        let decls = parse_decls(
            ".x { transition: left 120ms cubic-bezier(0.22, 0.61, 0.36, 1), background 120ms cubic-bezier(0.22, 0.61, 0.36, 1); }",
        );
        let defs = decls
            .iter()
            .find_map(|d| match d {
                StyleDeclaration::Transition(v) => Some(v),
                _ => None,
            })
            .expect("declaration with an unanimatable property must not drop");

        // Only the `background` entry survives; `left` is silently skipped.
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].property, TransitionProperty::Background);
        assert!((defs[0].duration.as_secs_f32() - 0.12).abs() < 0.01);
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
    fn test_inset_two_values() {
        let decls = parse_decls(".x { inset: 10px 20px; }");
        assert_eq!(decls.len(), 4);
        assert!(matches!(&decls[0], StyleDeclaration::Top(d) if *d == Dimension::Px(10.0)));
        assert!(matches!(&decls[1], StyleDeclaration::Right(d) if *d == Dimension::Px(20.0)));
        assert!(matches!(&decls[2], StyleDeclaration::Bottom(d) if *d == Dimension::Px(10.0)));
        assert!(matches!(&decls[3], StyleDeclaration::Left(d) if *d == Dimension::Px(20.0)));
    }

    #[test]
    fn test_inset_three_values() {
        let decls = parse_decls(".x { inset: 10px 20px 30px; }");
        assert_eq!(decls.len(), 4);
        assert!(matches!(&decls[0], StyleDeclaration::Top(d) if *d == Dimension::Px(10.0)));
        assert!(matches!(&decls[1], StyleDeclaration::Right(d) if *d == Dimension::Px(20.0)));
        assert!(matches!(&decls[2], StyleDeclaration::Bottom(d) if *d == Dimension::Px(30.0)));
        assert!(matches!(&decls[3], StyleDeclaration::Left(d) if *d == Dimension::Px(20.0)));
    }

    #[test]
    fn test_inset_four_values() {
        let decls = parse_decls(".x { inset: 10px 20px 30px 40px; }");
        assert_eq!(decls.len(), 4);
        assert!(matches!(&decls[0], StyleDeclaration::Top(d) if *d == Dimension::Px(10.0)));
        assert!(matches!(&decls[1], StyleDeclaration::Right(d) if *d == Dimension::Px(20.0)));
        assert!(matches!(&decls[2], StyleDeclaration::Bottom(d) if *d == Dimension::Px(30.0)));
        assert!(matches!(&decls[3], StyleDeclaration::Left(d) if *d == Dimension::Px(40.0)));
    }

    #[test]
    fn test_inset_auto_mixed() {
        let decls = parse_decls(".x { inset: auto 10px; }");
        assert_eq!(decls.len(), 4);
        assert!(matches!(&decls[0], StyleDeclaration::Top(d) if *d == Dimension::Auto));
        assert!(matches!(&decls[1], StyleDeclaration::Right(d) if *d == Dimension::Px(10.0)));
        assert!(matches!(&decls[2], StyleDeclaration::Bottom(d) if *d == Dimension::Auto));
        assert!(matches!(&decls[3], StyleDeclaration::Left(d) if *d == Dimension::Px(10.0)));
    }

    #[test]
    fn test_inset_percent() {
        let decls = parse_decls(".x { inset: 50%; }");
        assert_eq!(decls.len(), 4);
        let pct50 = Dimension::Percent(50.0);
        assert!(matches!(&decls[0], StyleDeclaration::Top(d) if *d == pct50));
        assert!(matches!(&decls[1], StyleDeclaration::Right(d) if *d == pct50));
        assert!(matches!(&decls[2], StyleDeclaration::Bottom(d) if *d == pct50));
        assert!(matches!(&decls[3], StyleDeclaration::Left(d) if *d == pct50));
    }

    #[test]
    fn test_parse_calc_vw_minus_px() {
        // The app's modal: `max-width: calc(100vw - 48px)`.
        let decls = parse_decls(".x { max-width: calc(100vw - 48px); }");
        assert_eq!(
            decls,
            vec![StyleDeclaration::MaxWidth(Dimension::Calc { px: -48.0, vw: 100.0, vh: 0.0 })]
        );
    }

    #[test]
    fn test_parse_calc_vh_minus_px() {
        let decls = parse_decls(".x { max-height: calc(72vh - 46px); }");
        assert_eq!(
            decls,
            vec![StyleDeclaration::MaxHeight(Dimension::Calc { px: -46.0, vw: 0.0, vh: 72.0 })]
        );
    }

    #[test]
    fn test_parse_max_size_none() {
        let decls = parse_decls(".x { max-width: none; max-height: none; }");
        assert_eq!(
            decls,
            vec![
                StyleDeclaration::MaxWidth(Dimension::Auto),
                StyleDeclaration::MaxHeight(Dimension::Auto)
            ]
        );
    }

    #[test]
    fn test_parse_none_stays_scoped_to_max_size() {
        let sheet = CompiledStylesheet::parse(".x { width: none; height: none; }");
        assert!(sheet.dropped.iter().any(|d| d.property == "width" && d.value == "none"));
        assert!(sheet.dropped.iter().any(|d| d.property == "height" && d.value == "none"));
    }

    #[test]
    fn test_parse_calc_with_parens_and_division() {
        // Precedence + parens: ((100vw - 48px) / 2) = 50vw - 24px.
        let decls = parse_decls(".x { width: calc((100vw - 48px) / 2); }");
        assert_eq!(
            decls,
            vec![StyleDeclaration::Width(Dimension::Calc { px: -24.0, vw: 50.0, vh: 0.0 })]
        );
    }

    #[test]
    fn test_parse_calc_pure_px_collapses_to_px() {
        // A px-only calc reduces to a plain Px (no Calc wrapper).
        let decls = parse_decls(".x { width: calc(20px + 4px); }");
        assert_eq!(decls, vec![StyleDeclaration::Width(Dimension::Px(24.0))]);
    }

    #[test]
    fn test_parse_calc_constant_on_px_pathway() {
        // The margin form after var() resolution: `calc(12px * -1)` = -12px.
        // Margin uses the px pathway (parse_px), which folds constant calc.
        let decls = parse_decls(".x { margin: 0 calc(12px * -1); }");
        let m = decls
            .iter()
            .find_map(|d| match d {
                StyleDeclaration::Margin(e) => Some(*e),
                _ => None,
            })
            .expect("margin parsed");
        assert_eq!(m.right, -12.0);
        assert_eq!(m.left, -12.0);
        assert_eq!(m.top, 0.0);
    }

    #[test]
    fn test_parse_calc_percent_mix_rejected() {
        // `calc(100% - 14px)` mixes percent + length, which taffy cannot
        // represent — it must drop rather than mis-render.
        let sheet = CompiledStylesheet::parse(".x { width: calc(100% - 14px); }");
        assert!(
            sheet.dropped.iter().any(|d| d.property == "width"),
            "a percent+length calc must drop"
        );
    }

    #[test]
    fn test_parse_vh_unit() {
        let decls = parse_decls(".x { max-height: 80vh; }");
        assert_eq!(decls.len(), 1);
        assert!(
            matches!(&decls[0], StyleDeclaration::MaxHeight(d) if *d == Dimension::Vh(80.0)),
            "80vh should parse to Dimension::Vh(80.0), got {:?}",
            decls[0]
        );
    }

    #[test]
    fn test_parse_vw_unit() {
        let decls = parse_decls(".x { max-width: 50vw; }");
        assert_eq!(decls.len(), 1);
        assert!(
            matches!(&decls[0], StyleDeclaration::MaxWidth(d) if *d == Dimension::Vw(50.0)),
            "50vw should parse to Dimension::Vw(50.0), got {:?}",
            decls[0]
        );
    }

    #[test]
    fn test_parse_vh_vw_fractional() {
        let decls = parse_decls(".x { width: 33.5vw; height: 66.7vh; }");
        assert_eq!(decls.len(), 2);
        assert!(matches!(
            &decls[0],
            StyleDeclaration::Width(d) if *d == Dimension::Vw(33.5)
        ));
        assert!(matches!(
            &decls[1],
            StyleDeclaration::Height(d) if *d == Dimension::Vh(66.7)
        ));
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
        // var(--accent) in a stop color now defers; the cascade resolves it
        // against the base env, substitutes the expanded value into the gradient
        // text, and the gradient parser runs on the concrete text.
        let css = r#"
            :root { --accent: #ff00aa; }
            .x { background: linear-gradient(90deg, transparent, var(--accent) 50%, transparent); }
        "#;
        let sheet = CompiledStylesheet::parse(css);
        let decls = rule_decls_for_class(&sheet, "x").to_vec();
        let style = apply_rule_with_base_env(&sheet, &decls);
        let g = match style.background {
            types::Background::LinearGradient(g) => g,
            other => panic!("expected LinearGradient, got {other:?}"),
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

    #[test]
    fn test_margin_left_auto_applies_auto_flag() {
        use crate::style::types::ComputedStyle;

        let decls = parse_decls(".x { margin-left: auto; }");
        assert!(decls.iter().any(|d| matches!(d, StyleDeclaration::MarginLeftAuto)));

        let mut style = ComputedStyle::default();
        for decl in &decls {
            apply_declaration(&mut style, decl);
        }

        assert!(style.margin_auto.left);
        assert_eq!(style.margin.left, 0.0);
    }

    #[test]
    fn test_margin_shorthand_keeps_mixed_auto_and_lengths() {
        use crate::style::types::ComputedStyle;

        let decls = parse_decls(".x { margin: 4px auto 8px 12px; }");
        let mut style = ComputedStyle::default();
        for decl in &decls {
            apply_declaration(&mut style, decl);
        }

        assert_eq!(style.margin.top, 4.0);
        assert_eq!(style.margin.bottom, 8.0);
        assert_eq!(style.margin.left, 12.0);
        assert!(style.margin_auto.right);
        assert!(!style.margin_auto.top);
        assert!(!style.margin_auto.bottom);
        assert!(!style.margin_auto.left);
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
    fn test_css_resize_all_values() {
        let cases = [
            ("none", types::CssResize::None),
            ("both", types::CssResize::Both),
            ("horizontal", types::CssResize::Horizontal),
            ("vertical", types::CssResize::Vertical),
        ];
        for (css_val, expected) in &cases {
            let decls = parse_decls(&format!(".x {{ resize: {}; }}", css_val));
            let val = decls.iter().find_map(|d| match d {
                StyleDeclaration::Resize(v) => Some(*v),
                _ => None,
            });
            assert_eq!(val, Some(*expected), "failed for resize: {}", css_val);
        }
    }

    #[test]
    fn test_css_resize_invalid() {
        let decls = parse_decls(".x { resize: magic; }");
        let val = decls.iter().find_map(|d| match d {
            StyleDeclaration::Resize(_) => Some(()),
            _ => None,
        });
        assert!(val.is_none());
    }

    #[test]
    fn test_object_fit_all_values() {
        let cases = [
            ("fill", types::ObjectFit::Fill),
            ("contain", types::ObjectFit::Contain),
            ("cover", types::ObjectFit::Cover),
            ("none", types::ObjectFit::None),
            ("scale-down", types::ObjectFit::ScaleDown),
        ];
        for (css_val, expected) in &cases {
            let decls = parse_decls(&format!(".x {{ object-fit: {}; }}", css_val));
            let val = decls.iter().find_map(|d| match d {
                StyleDeclaration::ObjectFit(v) => Some(*v),
                _ => None,
            });
            assert_eq!(val, Some(*expected), "failed for object-fit: {}", css_val);
        }
    }

    #[test]
    fn test_object_position_center() {
        let decls = parse_decls(".x { object-position: center; }");
        let val = decls.iter().find_map(|d| match d {
            StyleDeclaration::ObjectPosition(v) => Some(*v),
            _ => None,
        });
        let pos = val.unwrap();
        assert!((pos.x - 50.0).abs() < 0.01);
        assert!((pos.y - 50.0).abs() < 0.01);
    }

    #[test]
    fn test_object_position_keywords() {
        let decls = parse_decls(".x { object-position: left top; }");
        let val = decls.iter().find_map(|d| match d {
            StyleDeclaration::ObjectPosition(v) => Some(*v),
            _ => None,
        });
        let pos = val.unwrap();
        assert!((pos.x - 0.0).abs() < 0.01);
        assert!((pos.y - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_object_position_percentages() {
        let decls = parse_decls(".x { object-position: 25% 75%; }");
        let val = decls.iter().find_map(|d| match d {
            StyleDeclaration::ObjectPosition(v) => Some(*v),
            _ => None,
        });
        let pos = val.unwrap();
        assert!((pos.x - 25.0).abs() < 0.01);
        assert!((pos.y - 75.0).abs() < 0.01);
    }

    #[test]
    fn test_transition_min_max_properties() {
        let cases = [
            ("min-width", TransitionProperty::MinWidth),
            ("max-width", TransitionProperty::MaxWidth),
            ("min-height", TransitionProperty::MinHeight),
            ("max-height", TransitionProperty::MaxHeight),
        ];
        for (css_val, expected) in &cases {
            let decls = parse_decls(&format!(".x {{ transition: {} 0.3s ease; }}", css_val));
            let defs = decls
                .iter()
                .find_map(|d| match d {
                    StyleDeclaration::Transition(v) => Some(v),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("should have transition for {}", css_val));
            assert_eq!(defs.len(), 1, "wrong count for {}", css_val);
            assert_eq!(defs[0].property, *expected, "wrong property for {}", css_val);
        }
    }

    #[test]
    fn test_box_sizing_content_box() {
        let decls = parse_decls(".x { box-sizing: content-box; }");
        let val = decls.iter().find_map(|d| match d {
            StyleDeclaration::BoxSizing(v) => Some(*v),
            _ => None,
        });
        assert_eq!(val, Some(types::BoxSizing::ContentBox));
    }

    #[test]
    fn test_box_sizing_border_box() {
        let decls = parse_decls(".x { box-sizing: border-box; }");
        let val = decls.iter().find_map(|d| match d {
            StyleDeclaration::BoxSizing(v) => Some(*v),
            _ => None,
        });
        assert_eq!(val, Some(types::BoxSizing::BorderBox));
    }

    #[test]
    fn test_box_sizing_invalid() {
        let decls = parse_decls(".x { box-sizing: magic; }");
        let val = decls.iter().find_map(|d| match d {
            StyleDeclaration::BoxSizing(_) => Some(()),
            _ => None,
        });
        assert!(val.is_none());
    }

    #[test]
    fn test_aspect_ratio_auto() {
        let decls = parse_decls(".x { aspect-ratio: auto; }");
        let val = decls.iter().find_map(|d| match d {
            StyleDeclaration::AspectRatio(v) => Some(*v),
            _ => None,
        });
        assert_eq!(val, Some(None));
    }

    #[test]
    fn test_aspect_ratio_single_number() {
        let decls = parse_decls(".x { aspect-ratio: 1.5; }");
        let val = decls.iter().find_map(|d| match d {
            StyleDeclaration::AspectRatio(v) => Some(*v),
            _ => None,
        });
        assert_eq!(val, Some(Some(1.5)));
    }

    #[test]
    fn test_aspect_ratio_fraction() {
        let decls = parse_decls(".x { aspect-ratio: 16 / 9; }");
        let val = decls.iter().find_map(|d| match d {
            StyleDeclaration::AspectRatio(v) => Some(*v),
            _ => None,
        });
        let ratio = val.unwrap().unwrap();
        assert!((ratio - 16.0 / 9.0).abs() < 0.001);
    }

    #[test]
    fn test_aspect_ratio_square() {
        let decls = parse_decls(".x { aspect-ratio: 1 / 1; }");
        let val = decls.iter().find_map(|d| match d {
            StyleDeclaration::AspectRatio(v) => Some(*v),
            _ => None,
        });
        assert_eq!(val, Some(Some(1.0)));
    }

    #[test]
    fn test_directional_resize_cursors_parse() {
        let cases = [
            ("n-resize", CursorStyle::NResize),
            ("s-resize", CursorStyle::SResize),
            ("e-resize", CursorStyle::EResize),
            ("w-resize", CursorStyle::WResize),
            ("ne-resize", CursorStyle::NeResize),
            ("nw-resize", CursorStyle::NwResize),
            ("se-resize", CursorStyle::SeResize),
            ("sw-resize", CursorStyle::SwResize),
            ("ns-resize", CursorStyle::NsResize),
            ("ew-resize", CursorStyle::EwResize),
            ("nesw-resize", CursorStyle::NeswResize),
            ("nwse-resize", CursorStyle::NwseResize),
        ];
        for (css_val, expected) in &cases {
            let decls = parse_decls(&format!(".x {{ cursor: {}; }}", css_val));
            let cursor = decls.iter().find_map(|d| match d {
                StyleDeclaration::Cursor(v) => Some(*v),
                _ => None,
            });
            assert_eq!(cursor, Some(*expected), "failed for cursor: {}", css_val);
        }
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

    #[test]
    fn test_app_region_values() {
        let values =
            [("auto", AppRegion::Auto), ("drag", AppRegion::Drag), ("no-drag", AppRegion::NoDrag)];
        for (css_val, expected) in values {
            let css = format!(".x {{ -webkit-app-region: {}; }}", css_val);
            let decls = parse_decls(&css);
            let region = decls.iter().find_map(|d| match d {
                StyleDeclaration::AppRegion(v) => Some(*v),
                _ => None,
            });
            assert_eq!(region, Some(expected), "-webkit-app-region: {css_val}");
        }
    }

    #[test]
    fn test_app_region_applies_to_computed_style() {
        let decls = parse_decls(".x { -webkit-app-region: no-drag; }");
        let mut style = ComputedStyle::default();
        for decl in &decls {
            apply_declaration(&mut style, decl);
        }

        assert_eq!(style.app_region, AppRegion::NoDrag);
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

    #[test]
    fn test_of_type_structural_selectors_parse() {
        let first = last_parts_of(".cell:first-of-type { border-left: none; }");
        assert!(
            first.iter().any(|p| matches!(p, SelectorPart::PseudoClass(PseudoClass::FirstOfType))),
            "should parse :first-of-type pseudo-class"
        );

        let last = last_parts_of(".cell:last-of-type { border-right: none; }");
        assert!(
            last.iter().any(|p| matches!(p, SelectorPart::PseudoClass(PseudoClass::LastOfType))),
            "should parse :last-of-type pseudo-class"
        );
    }

    #[test]
    fn test_unknown_pseudo_class_rejects_selector() {
        let sheet = CompiledStylesheet::parse(".cell:unsupported { color: #ff0000; }");
        assert_eq!(
            sheet.rules.len(),
            0,
            "unknown pseudo classes must not leak to the base selector"
        );
    }

    #[test]
    fn test_z_index_positive() {
        let decls = parse_decls(".x { z-index: 10; }");
        let zi = decls.iter().find_map(|d| match d {
            StyleDeclaration::ZIndex(v) => Some(*v),
            _ => None,
        });
        assert_eq!(zi, Some(10));
    }

    #[test]
    fn test_z_index_negative() {
        let decls = parse_decls(".x { z-index: -5; }");
        let zi = decls.iter().find_map(|d| match d {
            StyleDeclaration::ZIndex(v) => Some(*v),
            _ => None,
        });
        assert_eq!(zi, Some(-5));
    }

    #[test]
    fn test_z_index_zero() {
        let decls = parse_decls(".x { z-index: 0; }");
        let zi = decls.iter().find_map(|d| match d {
            StyleDeclaration::ZIndex(v) => Some(*v),
            _ => None,
        });
        assert_eq!(zi, Some(0));
    }

    #[test]
    fn test_z_index_applied_to_computed_style() {
        let decls = parse_decls(".x { z-index: 42; }");
        let mut style = ComputedStyle::default();
        for d in &decls {
            apply_declaration(&mut style, d);
        }
        assert_eq!(style.z_index, 42);
    }

    #[test]
    fn test_position_fixed_parsed() {
        let decls = parse_decls(".x { position: fixed; }");
        let pos = decls.iter().find_map(|d| match d {
            StyleDeclaration::Position(p) => Some(*p),
            _ => None,
        });
        assert_eq!(
            pos,
            Some(CssPosition::Fixed),
            "position: fixed should parse to CssPosition::Fixed"
        );
    }

    #[test]
    fn test_position_fixed_maps_to_absolute_in_taffy() {
        use crate::style::types::ComputedStyle;

        let mut style = ComputedStyle::default();
        style.position = CssPosition::Fixed;
        let taffy = style.to_taffy_style(800.0, 600.0);
        assert_eq!(
            taffy.position,
            taffy::Position::Absolute,
            "CssPosition::Fixed should map to taffy::Position::Absolute"
        );
    }

    // -----------------------------------------------------------------
    // `oklch()` color function parsing.
    //
    // The Organiza Nota Wireframes v1 and v2 palettes are defined in
    // oklch, e.g. `oklch(0.65 0.17 145)` for the stamp-green accent.
    // Before support landed, these calls produced a parse error and the
    // variables fell back to the default color, silently breaking the
    // wireframe theme.
    // -----------------------------------------------------------------

    fn parse_single_color(css: &str) -> Option<Color> {
        let mut input = ParserInput::new(css);
        let mut parser = Parser::new(&mut input);
        parse_color(&mut parser).ok()
    }

    #[test]
    fn oklch_zero_chroma_at_full_lightness_is_white() {
        let c = parse_single_color("oklch(1.0 0.0 0.0)").expect("oklch parse");
        assert_eq!(
            (c.r, c.g, c.b, c.a),
            (255, 255, 255, 255),
            "oklch(1.0 0.0 0.0) must round-trip to pure white"
        );
    }

    #[test]
    fn oklch_zero_lightness_is_black() {
        let c = parse_single_color("oklch(0.0 0.0 0.0)").expect("oklch parse");
        assert_eq!(
            (c.r, c.g, c.b, c.a),
            (0, 0, 0, 255),
            "oklch(0.0 0.0 0.0) must round-trip to pure black"
        );
    }

    #[test]
    fn oklch_with_chroma_produces_hue_colored_output() {
        // L=0.65, C=0.17, H=145deg is the wireframes v1 stamp green. The
        // exact sRGB output depends on gamma conversion; assert the hue
        // signature: green dominates over red and blue.
        let c = parse_single_color("oklch(0.65 0.17 145)").expect("oklch parse");
        assert!(
            c.g > c.r && c.g > c.b,
            "oklch(0.65 0.17 145) should have green dominance, got rgba({}, {}, {}, {})",
            c.r,
            c.g,
            c.b,
            c.a
        );
    }

    #[test]
    fn oklch_alpha_slash_syntax_populates_alpha_channel() {
        let c = parse_single_color("oklch(0.65 0.17 40 / 0.5)").expect("oklch parse");
        // 0.5 alpha rounds to 128 (0.5 * 255 = 127.5, banker's round to 128).
        assert!(
            (c.a as i32 - 128).abs() <= 1,
            "oklch(... / 0.5) must set alpha near 128, got {}",
            c.a
        );
    }

    #[test]
    fn oklch_lightness_percentage_equals_unit_number() {
        let pct = parse_single_color("oklch(50% 0.0 0.0)").expect("percent parse");
        let num = parse_single_color("oklch(0.5 0.0 0.0)").expect("number parse");
        assert_eq!(
            (pct.r, pct.g, pct.b),
            (num.r, num.g, num.b),
            "oklch(50% ...) must equal oklch(0.5 ...) modulo rounding"
        );
    }

    #[test]
    fn oklch_chroma_percentage_equals_number_scaled_by_0_4() {
        // CSS Color Level 4 says C of 100% equals 0.4 numeric. So
        // oklch(0.65 50% 145) must equal oklch(0.65 0.2 145).
        let pct = parse_single_color("oklch(0.65 50% 145)").expect("percent parse");
        let num = parse_single_color("oklch(0.65 0.2 145)").expect("number parse");
        assert_eq!((pct.r, pct.g, pct.b), (num.r, num.g, num.b), "oklch chroma 50% must equal 0.2");
    }

    #[test]
    fn oklch_hue_angle_deg_keyword_accepted() {
        let no_unit = parse_single_color("oklch(0.65 0.17 145)").expect("bare hue parse");
        let deg = parse_single_color("oklch(0.65 0.17 145deg)").expect("deg hue parse");
        assert_eq!(
            (no_unit.r, no_unit.g, no_unit.b),
            (deg.r, deg.g, deg.b),
            "bare hue number and deg angle must match"
        );
    }

    #[test]
    fn oklch_without_alpha_slash_is_opaque() {
        let c = parse_single_color("oklch(0.5 0.1 60)").expect("oklch parse");
        assert_eq!(c.a, 255, "oklch without alpha must be fully opaque");
    }

    #[test]
    fn oklch_rejects_malformed_input() {
        assert!(parse_single_color("oklch()").is_none(), "oklch() is invalid");
        assert!(parse_single_color("oklch(0.5)").is_none(), "oklch missing C and H is invalid");
        assert!(parse_single_color("oklch(0.5 0.1)").is_none(), "oklch missing H is invalid");
    }

    #[test]
    fn oklch_clamps_lightness_above_one_to_white() {
        let over = parse_single_color("oklch(2.0 0.0 0)").expect("over-range L parses");
        let clamped = parse_single_color("oklch(1.0 0.0 0)").expect("boundary L parses");
        assert_eq!(
            (over.r, over.g, over.b),
            (clamped.r, clamped.g, clamped.b),
            "L above 1.0 must clamp to 1.0 (white)"
        );
    }

    #[test]
    fn oklch_clamps_negative_lightness_to_black() {
        let neg = parse_single_color("oklch(-0.5 0.0 0)").expect("negative L parses");
        let clamped = parse_single_color("oklch(0.0 0.0 0)").expect("boundary L parses");
        assert_eq!(
            (neg.r, neg.g, neg.b),
            (clamped.r, clamped.g, clamped.b),
            "L below 0.0 must clamp to 0.0 (black)"
        );
    }

    #[test]
    fn oklch_clamps_negative_chroma_to_zero() {
        let neg = parse_single_color("oklch(0.5 -0.1 60)").expect("negative C parses");
        let clamped = parse_single_color("oklch(0.5 0.0 60)").expect("zero C parses");
        assert_eq!(
            (neg.r, neg.g, neg.b),
            (clamped.r, clamped.g, clamped.b),
            "negative chroma must clamp to 0 (grayscale)"
        );
    }

    #[test]
    fn oklch_clamps_over_range_alpha() {
        let over = parse_single_color("oklch(0.5 0.1 60 / 2.0)").expect("over-range alpha parses");
        assert_eq!(over.a, 255, "alpha above 1.0 must clamp to fully opaque");

        let neg = parse_single_color("oklch(0.5 0.1 60 / -0.5)").expect("negative alpha parses");
        assert_eq!(neg.a, 0, "alpha below 0.0 must clamp to fully transparent");
    }

    #[test]
    fn oklch_alpha_percentage_equals_unit_number() {
        let pct = parse_single_color("oklch(0.5 0.1 60 / 50%)").expect("percent alpha parses");
        let num = parse_single_color("oklch(0.5 0.1 60 / 0.5)").expect("number alpha parses");
        assert_eq!(pct.a, num.a, "50% alpha must equal 0.5 numeric");
    }

    #[test]
    fn oklch_hue_in_radians_matches_equivalent_degrees() {
        // pi/4 rad = 45 deg.
        let rad = parse_single_color("oklch(0.6 0.12 0.7853982rad)").expect("rad hue parses");
        let deg = parse_single_color("oklch(0.6 0.12 45)").expect("deg hue parses");
        assert!(
            (rad.r as i32 - deg.r as i32).abs() <= 1
                && (rad.g as i32 - deg.g as i32).abs() <= 1
                && (rad.b as i32 - deg.b as i32).abs() <= 1,
            "pi/4 rad and 45 deg must round to within 1 sRGB unit"
        );
    }

    #[test]
    fn oklch_hue_in_gradians_matches_equivalent_degrees() {
        // 100 grad = 90 deg.
        let grad = parse_single_color("oklch(0.6 0.12 100grad)").expect("grad hue parses");
        let deg = parse_single_color("oklch(0.6 0.12 90)").expect("deg hue parses");
        assert!(
            (grad.r as i32 - deg.r as i32).abs() <= 1
                && (grad.g as i32 - deg.g as i32).abs() <= 1
                && (grad.b as i32 - deg.b as i32).abs() <= 1,
            "100 grad and 90 deg must round to within 1 sRGB unit"
        );
    }

    #[test]
    fn oklch_hue_in_turns_matches_equivalent_degrees() {
        // 0.25 turn = 90 deg.
        let turn = parse_single_color("oklch(0.6 0.12 0.25turn)").expect("turn hue parses");
        let deg = parse_single_color("oklch(0.6 0.12 90)").expect("deg hue parses");
        assert!(
            (turn.r as i32 - deg.r as i32).abs() <= 1
                && (turn.g as i32 - deg.g as i32).abs() <= 1
                && (turn.b as i32 - deg.b as i32).abs() <= 1,
            "0.25 turn and 90 deg must round to within 1 sRGB unit"
        );
    }

    #[test]
    fn oklch_rejects_unknown_hue_angle_unit() {
        assert!(
            parse_single_color("oklch(0.5 0.1 60foo)").is_none(),
            "unknown hue angle unit must be rejected"
        );
    }

    #[test]
    fn parse_px_rejects_vh_and_vw() {
        // The px pathway is used for padding, border-width, gap, etc. It
        // has no viewport context and cannot resolve viewport units, so
        // `padding: 5vh` must fail to parse rather than silently becoming
        // `padding: 5px`. See `parse_px`.
        let mut input = ParserInput::new("5vh");
        let mut parser = Parser::new(&mut input);
        assert!(parse_px(&mut parser).is_err(), "parse_px must reject 5vh");

        let mut input = ParserInput::new("5vw");
        let mut parser = Parser::new(&mut input);
        assert!(parse_px(&mut parser).is_err(), "parse_px must reject 5vw");
    }

    #[test]
    fn grid_track_parsers_reject_vh_and_vw() {
        // Grid track sizes likewise have no viewport context at parse
        // time. Letting `grid-template-rows: 50vh` silently degrade to
        // `50px` would misposition rows at any non-100px viewport. Pin
        // rejection across all four grid entry points.
        for css in ["50vh", "50vw"] {
            let mut input = ParserInput::new(css);
            let mut parser = Parser::new(&mut input);
            assert!(
                parse_grid_track_size_single(&mut parser).is_err(),
                "parse_grid_track_size_single must reject {}",
                css
            );

            let mut input = ParserInput::new(css);
            let mut parser = Parser::new(&mut input);
            assert!(
                parse_grid_min_track_size(&mut parser).is_err(),
                "parse_grid_min_track_size must reject {}",
                css
            );

            let mut input = ParserInput::new(css);
            let mut parser = Parser::new(&mut input);
            assert!(
                parse_grid_max_track_size(&mut parser).is_err(),
                "parse_grid_max_track_size must reject {}",
                css
            );
        }

        // fit-content(50vh) lives inside parse_grid_function_track, which
        // is reached via parse_grid_track_size_single.
        for css in ["fit-content(50vh)", "fit-content(50vw)"] {
            let mut input = ParserInput::new(css);
            let mut parser = Parser::new(&mut input);
            assert!(
                parse_grid_track_size_single(&mut parser).is_err(),
                "fit-content must reject {}",
                css
            );
        }
    }

    #[test]
    fn oklch_leaves_trailing_tokens_for_caller() {
        // parse_color consumes one color literal and stops. Trailing
        // tokens past the closing `)` are the caller's responsibility to
        // validate via expect_exhausted. Pin this contract so a future
        // refactor that tightens parse_color doesn't silently break
        // callers that compose a color with other productions.
        assert!(
            parse_single_color("oklch(0.5 0.1 60) garbage").is_some(),
            "parse_color must stop at the oklch() call and leave trailing tokens untouched"
        );
    }

    // --- parse-arms cluster -------------------------------------------------

    #[test]
    fn outline_shorthand_is_order_independent() {
        // width color style, in canonical and shuffled order.
        for css in [
            ".x { outline: 1px solid #abcdef; }",
            ".x { outline: solid #abcdef 1px; }",
            ".x { outline: #abcdef 1px solid; }",
        ] {
            let decls = parse_decls(css);
            assert!(
                decls.contains(&StyleDeclaration::OutlineWidth(1.0)),
                "{css} should yield outline-width 1px: {decls:?}"
            );
            assert!(
                decls.iter().any(|d| matches!(d, StyleDeclaration::OutlineColor(_))),
                "{css} should yield an outline-color: {decls:?}"
            );
        }
    }

    #[test]
    fn outline_none_collapses_to_zero_width() {
        let decls = parse_decls(".x { outline: none; }");
        assert_eq!(decls, vec![StyleDeclaration::OutlineWidth(0.0)]);
    }

    #[test]
    fn outline_resolves_var_color_from_stylesheet() {
        // Mirrors the real stylesheet form `outline: 1px solid var(...)`. The
        // `var(`-bearing value now defers and resolves in the cascade against the
        // base scope, so resolve through the base-env apply rather than expecting
        // a typed declaration straight out of `parse()`.
        let sheet = CompiledStylesheet::parse(
            ".x { outline: 1px solid var(--border-focus); } :root { --border-focus: #112233; }",
        );
        let decls = rule_decls_for_class(&sheet, "x").to_vec();
        let style = apply_rule_with_base_env(&sheet, &decls);
        assert!(
            (style.outline_width - 1.0).abs() < 0.01,
            "outline-width 1px, got {}",
            style.outline_width
        );
        assert_eq!(style.outline_color, Color::rgb(0x11, 0x22, 0x33));
    }

    #[test]
    fn outline_style_only_defaults_width_to_zero() {
        // No explicit width and a non-none style keyword -> width 0.
        let decls = parse_decls(".x { outline: solid #abcdef; }");
        assert!(decls.contains(&StyleDeclaration::OutlineWidth(0.0)), "{decls:?}");
        assert!(decls.iter().any(|d| matches!(d, StyleDeclaration::OutlineColor(_))));
    }

    #[test]
    fn background_none_is_transparent() {
        let decls = parse_decls(".x { background: none; }");
        assert_eq!(
            decls,
            vec![StyleDeclaration::Background(types::Background::Color(Color::TRANSPARENT))]
        );
    }

    #[test]
    fn background_multi_layer_keeps_first_paintable() {
        // Two radial gradient layers: keep the first, drain the rest.
        let decls = parse_decls(
            ".x { background: radial-gradient(circle at 0% 0%, #ff0000, transparent), \
             radial-gradient(circle at 100% 100%, #00ff00, transparent); }",
        );
        assert_eq!(decls.len(), 1, "only one background layer is retained: {decls:?}");
        assert!(matches!(
            decls[0],
            StyleDeclaration::Background(types::Background::RadialGradient(_))
        ));
    }

    #[test]
    fn justify_content_stretch_parses() {
        let decls = parse_decls(".x { justify-content: stretch; }");
        assert_eq!(decls, vec![StyleDeclaration::JustifyContent(JustifyContent::Stretch)]);
    }

    #[test]
    fn justify_content_left_right_alias_to_start_end() {
        assert_eq!(
            parse_decls(".x { justify-content: left; }"),
            vec![StyleDeclaration::JustifyContent(JustifyContent::Start)]
        );
        assert_eq!(
            parse_decls(".x { justify-content: right; }"),
            vec![StyleDeclaration::JustifyContent(JustifyContent::End)]
        );
    }

    #[test]
    fn border_style_none_collapses_width() {
        assert_eq!(
            parse_decls(".x { border-style: none; }"),
            vec![StyleDeclaration::BorderWidth(Edges::all(0.0))]
        );
        assert_eq!(
            parse_decls(".x { border-style: hidden; }"),
            vec![StyleDeclaration::BorderWidth(Edges::all(0.0))]
        );
    }

    #[test]
    fn border_style_line_styles_are_accepted_but_inert() {
        // Accepted (no longer drops) but yields no declaration.
        assert_eq!(parse_decls(".x { border-style: solid; }"), Vec::<StyleDeclaration>::new());
        assert_eq!(parse_decls(".x { border-style: dashed; }"), Vec::<StyleDeclaration>::new());
    }

    #[test]
    fn border_style_garbage_still_drops() {
        let sheet = CompiledStylesheet::parse(".x { border-style: bogus; }");
        assert!(
            sheet.dropped.iter().any(|d| d.property == "border-style"),
            "an invalid border-style keyword must still drop"
        );
    }

    fn get_text_shadow_list(decls: &[StyleDeclaration]) -> SmallVec<[ParsedTextShadow; 2]> {
        decls
            .iter()
            .find_map(|d| match d {
                StyleDeclaration::TextShadowList(v) => Some(v.clone()),
                _ => None,
            })
            .expect("declaration should contain a text-shadow list")
    }

    #[test]
    fn text_shadow_none_clears_to_empty_list() {
        // `none` is a real (empty) list declaration that clears any earlier
        // text-shadow, mirroring `box-shadow: none`.
        let list = get_text_shadow_list(&parse_decls(".x { text-shadow: none; }"));
        assert!(list.is_empty());
    }

    #[test]
    fn text_shadow_glow_parses() {
        // The app's workspace-name glow: a zero-offset blurred glow.
        let list = get_text_shadow_list(&parse_decls(
            ".x { text-shadow: 0 0 8px rgba(246,217,136,0.2); }",
        ));
        assert_eq!(list.len(), 1);
        let s = list[0];
        assert_eq!((s.offset_x, s.offset_y, s.blur_radius), (0.0, 0.0, 8.0));
        let c = s.color.expect("explicit color should parse");
        assert_eq!((c.r, c.g, c.b), (246, 217, 136));
    }

    #[test]
    fn text_shadow_color_may_lead_or_be_omitted() {
        // Color-first form is valid CSS.
        let lead = get_text_shadow_list(&parse_decls(".x { text-shadow: #ff0000 1px 2px 3px; }"));
        assert_eq!(lead.len(), 1);
        assert_eq!((lead[0].offset_x, lead[0].offset_y, lead[0].blur_radius), (1.0, 2.0, 3.0));
        assert_eq!(lead[0].color.map(|c| (c.r, c.g, c.b)), Some((255, 0, 0)));
        // Omitted color stays None (resolved to currentColor at apply).
        let bare = get_text_shadow_list(&parse_decls(".x { text-shadow: 1px 1px; }"));
        assert_eq!(bare.len(), 1);
        assert_eq!(bare[0].blur_radius, 0.0);
        assert!(bare[0].color.is_none());
    }

    #[test]
    fn text_shadow_omitted_color_defaults_to_current_color() {
        let mut style = ComputedStyle::default();
        // The default `color` is opaque black; an omitted shadow color must
        // resolve to it (CSS `currentColor`).
        for d in &parse_decls(".x { text-shadow: 0 0 4px; }") {
            apply_declaration(&mut style, d);
        }
        assert_eq!(style.text_shadow.len(), 1);
        assert_eq!(style.text_shadow[0].color, style.color);
        assert_eq!(style.text_shadow[0].blur_radius, 4.0);
    }

    #[test]
    fn inert_noop_accepts_are_recognized_but_empty() {
        for css in [
            ".x { appearance: none; }",
            ".x { -webkit-appearance: none; }",
            ".x { -webkit-font-smoothing: antialiased; }",
            ".x { border-collapse: collapse; }",
            ".x { background-repeat: no-repeat; }",
            ".x { font-feature-settings: 'calt' 1, 'liga' 1; }",
            ".x { font-variant-numeric: tabular-nums; }",
            ".x { scrollbar-width: none; }",
        ] {
            let sheet = CompiledStylesheet::parse(css);
            assert!(
                sheet.dropped.iter().all(|d| d.is_custom_property()),
                "{css} should be accepted (not dropped): {:?}",
                sheet.dropped
            );
            assert_eq!(
                parse_decls(css),
                Vec::<StyleDeclaration>::new(),
                "{css} should yield no declarations"
            );
        }
    }

    // --- Stage 1: cascade-aware token-scope collection (additive) ------------

    #[test]
    fn token_scopes_collapse_root_and_star_into_base() {
        // `:root` and `*` both feed scope 0; later declarations override.
        let css = r#"
            :root { --a: 1; --b: 2; }
            * { --b: 3; --c: 4; }
            .x { --a: 9; }
        "#;
        let scopes = collect_token_scopes(css);
        let base = scopes.base().expect("a base scope");
        assert_eq!(base.key, ScopeKey(0));
        assert_eq!(base.selector_text, ":root");
        assert_eq!(base.vars.get("--a").map(String::as_str), Some("1"));
        // `*`'s --b overrides :root's --b (later block wins on merge).
        assert_eq!(base.vars.get("--b").map(String::as_str), Some("3"));
        assert_eq!(base.vars.get("--c").map(String::as_str), Some("4"));
        // `.x` is its own scope, not merged into base.
        let x = scopes.by_selector(".x").expect(".x scope");
        assert_eq!(x.vars.get("--a").map(String::as_str), Some("9"));
        assert!(base.vars.get("--a").map(String::as_str) != Some("9"));
        // Exactly two scopes: base + .x.
        assert_eq!(scopes.scopes.len(), 2);
    }

    #[test]
    fn token_scopes_record_specificity_and_source_order() {
        // source_order counts EVERY block, including the no-token `.plain` one,
        // so the third block (`#id.cls`) records source_order == 2.
        let css = r#"
            :root { --a: 1; }
            .plain { color: red; }
            #id.cls { --a: 2; }
        "#;
        let scopes = collect_token_scopes(css);
        let base = scopes.base().unwrap();
        // `:root` parses to a Universal part (see parse_simple_selector), which
        // contributes nothing to specificity.
        assert_eq!(base.specificity, (0, 0, 0));
        assert_eq!(base.source_order, 0);

        let scoped = scopes.by_selector("#id.cls").expect("#id.cls scope");
        // 1 id + 1 class.
        assert_eq!(scoped.specificity, (1, 1, 0));
        // Block index 2 (`:root`=0, `.plain`=1, `#id.cls`=2).
        assert_eq!(scoped.source_order, 2);
    }

    #[test]
    fn token_scopes_store_cross_token_refs_raw_not_preflattened() {
        // A base alias `--accent: var(--amber)` must be stored RAW (not eagerly
        // concretized to `#aaa`), so a theme that overrides `--amber` is seen by
        // every consumer reaching `--accent` through the alias. The resolution
        // is done lazily at use time against the element's `ScopeEnv`.
        let css = r#"
            :root { --amber: #aaa; --accent: var(--amber); }
            .theme { --amber: #bbb; }
        "#;
        let scopes = collect_token_scopes(css);
        let base = scopes.base().unwrap();
        // RAW: the cross-token reference is kept verbatim.
        assert_eq!(base.vars.get("--accent").map(String::as_str), Some("var(--amber)"));

        let theme = scopes.by_selector(".theme").unwrap();
        // The scope only declares --amber (concrete); stored as-is.
        assert_eq!(theme.vars.get("--amber").map(String::as_str), Some("#bbb"));

        // Use-time resolution: resolving `var(--accent)` against [theme, base]
        // must reach the THEME's --amber (#bbb), proving the two-level alias
        // propagates the theme override.
        let env = ScopeEnv::new(None, Some(theme.vars.as_ref()), Some(base.vars.as_ref()));
        assert_eq!(flatten_token_value_env("var(--accent)", &env), "#bbb");
        // With no theme active, it falls back to the base --amber (#aaa).
        let base_only = ScopeEnv::new(None, None, Some(base.vars.as_ref()));
        assert_eq!(flatten_token_value_env("var(--accent)", &base_only), "#aaa");
    }

    #[test]
    fn token_scopes_two_level_alias_propagates_theme_override() {
        // Mirrors styles.css: `:root` defines `--cp-accent: var(--amber-300)`,
        // a theme overrides ONLY `--amber-300` (NOT --cp-accent). A consumer of
        // `var(--cp-accent)` under the theme must resolve to the theme amber,
        // proving the override propagates through the base-scope alias.
        let css = r#"
            :root { --amber-300: #d4a348; --cp-accent: var(--amber-300); }
            .app.theme-dracula { --amber-300: #bd93f9; }
        "#;
        let scopes = collect_token_scopes(css);
        let base = scopes.base().unwrap();
        let dracula = scopes.by_selector(".app.theme-dracula").unwrap();
        // Stored raw.
        assert_eq!(base.vars.get("--cp-accent").map(String::as_str), Some("var(--amber-300)"));
        // Resolving `var(--cp-accent)` against [dracula, base] reaches the theme
        // amber even though dracula never redefines --cp-accent itself.
        let env = ScopeEnv::new(None, Some(dracula.vars.as_ref()), Some(base.vars.as_ref()));
        assert_eq!(
            flatten_token_value_env("var(--cp-accent)", &env),
            "#bd93f9",
            "theme override of the inner token must propagate through the base alias"
        );
    }

    #[test]
    fn token_scopes_preflatten_cycle_guard_terminates() {
        // A -> B -> A is a cycle; the flatten must terminate and leave the
        // unresolvable var() text in place rather than spin.
        let css = r#"
            :root { --a: var(--b); --b: var(--a); }
        "#;
        let scopes = collect_token_scopes(css);
        let base = scopes.base().unwrap();
        let a = base.vars.get("--a").map(String::as_str).unwrap();
        // Whatever the fixed point is, it still contains an unresolved var()
        // (no infinite loop, no panic).
        assert!(a.contains("var("), "cyclic token should retain a var(): got {a:?}");
    }

    #[test]
    fn token_scopes_empty_without_custom_props() {
        let css = ".a { color: red; } .b { display: flex; }";
        let scopes = collect_token_scopes(css);
        assert!(scopes.scopes.is_empty());
        assert!(scopes.base().is_none());
    }

    // ---- Defect 3: var( gate must only fire on a real var() function -------

    #[test]
    fn contains_var_function_matches_only_real_var_calls() {
        // Real var() functions at every value boundary.
        assert!(contains_var_function("var(--x)"));
        assert!(contains_var_function("  var(--x)"));
        assert!(contains_var_function("1px solid var(--x)"));
        assert!(contains_var_function("rgba(0,0,0,var(--a))")); // after '('
        assert!(contains_var_function("a, var(--x)")); // after ','
        assert!(contains_var_function("calc(var(--x) + 1px)"));
        // NOT a var() function: `var` is the tail of a longer identifier.
        assert!(!contains_var_function("myvar(1)"));
        assert!(!contains_var_function("url(.../myvar(1).png)"));
        assert!(!contains_var_function("foovar(--x)"));
        assert!(!contains_var_function("0px")); // no var at all
                                                // A real var() later in a string that also contains a fake one.
        assert!(contains_var_function("myvar(1) var(--x)"));
        assert!(!contains_var_function("url(myvar(1).png) novar(2)"));
    }

    #[test]
    fn fake_var_substring_is_not_deferred() {
        // `myvar(` is not a var() call, so the declaration must NOT take the
        // Deferred fast path — it falls through to the typed match (where this
        // unsupported `background` form simply errors). Either way, no Deferred
        // carrier is produced. Call `parse_declaration` directly so an Err on the
        // typed path is tolerated; the assertion is purely "not Deferred".
        let mut input = ParserInput::new("background: url(./myvar(1).png)");
        let mut parser = Parser::new(&mut input);
        let result = parse_declaration(&mut parser, ScopeKey(0));
        let has_deferred = result
            .map(|decls| decls.iter().any(|d| matches!(d, StyleDeclaration::Deferred { .. })))
            .unwrap_or(false);
        assert!(!has_deferred, "a 'myvar(' / url(... myvar(...)) substring must not be deferred");
    }

    // ---- Defect 4: self-scope perf gate -----------------------------------

    #[test]
    fn widget_scope_classes_gate_skips_non_widget_elements() {
        // `.theme-chip.dracula` is a widget scope keyed on classes; `.app` and
        // `.cp-mode-pill` are NOT widget-scope terminal classes here.
        let css = r#"
            :root { --x: 1; }
            .app.theme-dracula { --x: 2; }
            .theme-chip.dracula { --x: 3; }
        "#;
        let scopes = collect_token_scopes(css);
        // Terminal classes of the non-base scopes: theme-dracula, app, dracula,
        // theme-chip (the theme root's classes are over-included, harmless).
        assert!(scopes.widget_scope_classes.contains("theme-chip"));
        assert!(scopes.widget_scope_classes.contains("dracula"));
        assert!(!scopes.widget_scope_gate_unsafe, "all terminals carry a class");
        // An element with a widget class may have a self scope.
        assert!(scopes.element_may_have_self_scope(&["theme-chip".to_string()]));
        // An element with none of the widget classes is gated out.
        assert!(!scopes.element_may_have_self_scope(&["cp-mode-pill".to_string()]));
        assert!(!scopes.element_may_have_self_scope(&[]));
    }

    #[test]
    fn widget_scope_gate_disabled_for_classless_terminal_scope() {
        // An id-only scope terminal makes the class-intersection gate unsound, so
        // it must fall back to always running the self-scope walk.
        let css = r#"
            :root { --x: 1; }
            #widget { --x: 2; }
        "#;
        let scopes = collect_token_scopes(css);
        assert!(scopes.widget_scope_gate_unsafe, "an id-only terminal disables the gate");
        // Always returns true so the cascade never wrongly skips the walk.
        assert!(scopes.element_may_have_self_scope(&[]));
        assert!(scopes.element_may_have_self_scope(&["anything".to_string()]));
    }

    // ---- Defect 2: cascade-time var() failures reach the drop sink --------

    #[test]
    fn coverage_pass_records_malformed_scoped_var_into_dropped() {
        // A scoped var() that no scope defines and that has no fallback cannot
        // resolve under ANY env, so the parse-time coverage pass must record it
        // into `dropped` where the stylesheet_coverage guardrail can see it.
        let sheet = CompiledStylesheet::parse(
            ":root { --known: #00ff00; } .widget { color: var(--missing); }",
        );
        assert!(
            sheet
                .dropped
                .iter()
                .any(|d| d.property == "color" && d.value.contains("var(--missing)")),
            "an unresolvable scoped var() must reach the dropped sink: {:?}",
            sheet.dropped
        );
    }

    #[test]
    fn coverage_pass_does_not_drop_a_theme_resolvable_var() {
        // --accent is unresolvable under :root alone but IS defined by a theme
        // scope; resolving under that scope succeeds, so it must NOT be recorded
        // as a coverage failure.
        let sheet = CompiledStylesheet::parse(
            ".app.theme-dracula { --accent: #bd93f9; } .widget { color: var(--accent); }",
        );
        assert!(
            !sheet.dropped.iter().any(|d| d.property == "color"),
            "a var() that some theme scope resolves must not be flagged: {:?}",
            sheet.dropped
        );
    }

    // ---- Stage 2: deferred declaration carrier ----------------------------

    /// Parse a single `property: value` declaration via the scoped
    /// `parse_declaration` path directly, BYPASSING the global
    /// `resolve_var_references` pass that `CompiledStylesheet::parse` runs. This
    /// is the only way to reach a `var(`-carrying value with the current global
    /// resolve in place, so it exercises the `Deferred` carrier in isolation.
    fn parse_one_decl_scoped(text: &str, scope: ScopeKey) -> SmallVec<[StyleDeclaration; 2]> {
        let mut input = ParserInput::new(text);
        let mut parser = Parser::new(&mut input);
        parse_declaration(&mut parser, scope).expect("declaration should parse")
    }

    #[test]
    fn deferred_carrier_is_produced_when_global_resolve_bypassed() {
        // With the global resolve bypassed, a value that still contains `var(`
        // is captured verbatim as a Deferred carrier rather than typed eagerly.
        let decls = parse_one_decl_scoped("color: var(--accent)", ScopeKey(7));
        assert_eq!(
            decls.as_slice(),
            [StyleDeclaration::Deferred {
                property: "color".into(),
                raw_value: "var(--accent)".into(),
                scope_hint: ScopeKey(7),
            }]
        );
    }

    #[test]
    fn deferred_apply_reparses_to_same_typed_decl_as_eager_path() {
        // The eager path: a concrete `color: red` types to `Color(..)` and sets
        // `style.color`.
        let mut eager = ComputedStyle::default();
        for decl in &parse_one_decl_scoped("color: red", ScopeKey(0)) {
            apply_declaration(&mut eager, decl);
        }

        // The deferred path: `color: var(--accent)` captures a carrier, which
        // resolves `--accent: red` against :root and re-parses to the SAME
        // typed declaration, producing the SAME computed style.
        let deferred = parse_one_decl_scoped("color: var(--accent)", ScopeKey(0));
        let StyleDeclaration::Deferred { property, raw_value, scope_hint } = &deferred[0] else {
            panic!("expected a Deferred carrier, got {:?}", deferred[0]);
        };

        let mut props = HashMap::new();
        props.insert("--accent".to_string(), "red".to_string());
        let mut dropped = Vec::new();
        let mut applied = ComputedStyle::default();
        apply_deferred_declaration(
            &mut applied,
            property,
            raw_value,
            *scope_hint,
            &props,
            &mut dropped,
        );

        // Same resolved color, and nothing routed to dropped on the happy path.
        assert_eq!(applied.color, eager.color);
        assert!(dropped.is_empty(), "a resolvable deferred decl must not drop: {dropped:?}");

        // And the re-parse yields the exact same typed declaration the eager
        // path produced for `color: red`.
        let reparsed = parse_one_decl_scoped("color: red", ScopeKey(0));
        let resolved_concrete = parse_one_decl_scoped("color: red", ScopeKey(0));
        assert_eq!(reparsed.as_slice(), resolved_concrete.as_slice());
        assert!(matches!(reparsed.as_slice(), [StyleDeclaration::Color(_)]));
    }

    #[test]
    fn deferred_apply_routes_unresolvable_var_to_dropped() {
        // An unresolved `var(` (no matching token, no fallback) cannot be typed:
        // re-parse round-trips back to Deferred, so it must route to `dropped`
        // rather than be silently swallowed.
        let mut style = ComputedStyle::default();
        let mut dropped = Vec::new();
        let props = HashMap::new(); // empty: --missing resolves to nothing
        apply_deferred_declaration(
            &mut style,
            "color",
            "var(--missing)",
            ScopeKey(3),
            &props,
            &mut dropped,
        );
        assert_eq!(dropped.len(), 1, "unresolvable deferred must route to dropped");
        assert_eq!(dropped[0].property, "color");
    }

    #[test]
    fn deferred_apply_routes_unparseable_value_to_dropped() {
        // The var() resolves to a concrete value, but the value is not something
        // the property's parser accepts (an engine gap). It must route to
        // `dropped`, not be swallowed.
        let mut style = ComputedStyle::default();
        let mut dropped = Vec::new();
        let mut props = HashMap::new();
        props.insert("--bad".to_string(), "definitely-not-a-color".to_string());
        apply_deferred_declaration(
            &mut style,
            "color",
            "var(--bad)",
            ScopeKey(0),
            &props,
            &mut dropped,
        );
        assert_eq!(dropped.len(), 1, "unparseable resolved value must route to dropped");
        assert_eq!(dropped[0].property, "color");
    }

    #[test]
    fn no_var_value_is_typed_eagerly_not_deferred() {
        // The fast-check must not mis-route a plain value: the snapshot/reset
        // dance leaves a no-`var(` value typed exactly as before.
        let decls = parse_one_decl_scoped("display: flex", ScopeKey(0));
        assert_eq!(decls.as_slice(), [StyleDeclaration::Display(Display::Flex)]);
    }

    #[test]
    fn deferred_carrier_is_produced_in_production_parse_after_flip() {
        // Stage 3 deleted the global resolve, so a `var(`-bearing declaration now
        // reaches the parser with `var(` intact and is captured as a Deferred
        // carrier (resolved per element in the cascade). The carrier's scope_hint
        // points at the block it was authored in (here `.x`, which has no token
        // scope of its own, so the base scope 0).
        let sheet =
            CompiledStylesheet::parse(":root { --accent: #112233; } .x { color: var(--accent); }");
        let deferred: Vec<_> = sheet
            .rules
            .iter()
            .flat_map(|r| r.declarations.iter())
            .filter(|d| matches!(d, StyleDeclaration::Deferred { .. }))
            .collect();
        assert_eq!(deferred.len(), 1, "production parse must defer the var() declaration now");
        let StyleDeclaration::Deferred { property, raw_value, .. } = deferred[0] else {
            unreachable!()
        };
        assert_eq!(property.as_ref(), "color");
        assert_eq!(raw_value.as_ref(), "var(--accent)");

        // The custom-property DEFINITION `--accent` is collected, not dropped.
        assert!(
            !sheet.dropped.iter().any(|d| d.property == "--accent"),
            "custom-property definitions must not drop after the flip: {:?}",
            sheet.dropped
        );
    }

    // ---- Stage 3: env-aware deferred apply (per-scope resolution) ----------

    #[test]
    fn env_resolves_token_highest_specificity_first() {
        // The env is [self, root_theme, base]; a token defined in more than one
        // active scope resolves to the highest-specificity (earliest) hit.
        let base: HashMap<String, String> =
            [("--c".to_string(), "#000000".to_string())].into_iter().collect();
        let theme: HashMap<String, String> =
            [("--c".to_string(), "#111111".to_string())].into_iter().collect();
        let widget: HashMap<String, String> =
            [("--c".to_string(), "#bd93f9".to_string())].into_iter().collect();

        // Self scope present: it wins.
        let env = ScopeEnv::new(Some(&widget), Some(&theme), Some(&base));
        let mut style = ComputedStyle::default();
        let mut dropped = Vec::new();
        apply_deferred_against_env(
            &mut style,
            "color",
            "var(--c)",
            ScopeKey(2),
            &env,
            &mut dropped,
        );
        assert_eq!(style.color, Color::rgb(0xbd, 0x93, 0xf9));
        assert!(dropped.is_empty());

        // No self scope: the root theme wins over base.
        let env = ScopeEnv::new(None, Some(&theme), Some(&base));
        let mut style = ComputedStyle::default();
        apply_deferred_against_env(
            &mut style,
            "color",
            "var(--c)",
            ScopeKey(1),
            &env,
            &mut dropped,
        );
        assert_eq!(style.color, Color::rgb(0x11, 0x11, 0x11));

        // Only base: base value.
        let env = ScopeEnv::new(None, None, Some(&base));
        let mut style = ComputedStyle::default();
        apply_deferred_against_env(
            &mut style,
            "color",
            "var(--c)",
            ScopeKey(0),
            &env,
            &mut dropped,
        );
        assert_eq!(style.color, Color::rgb(0x00, 0x00, 0x00));
        assert!(dropped.is_empty());
    }

    #[test]
    fn env_uses_fallback_then_routes_unresolvable_to_dropped() {
        let base: HashMap<String, String> = HashMap::new();
        // Fallback is honored when no scope defines the name.
        let env = ScopeEnv::new(None, None, Some(&base));
        let mut style = ComputedStyle::default();
        let mut dropped = Vec::new();
        apply_deferred_against_env(
            &mut style,
            "color",
            "var(--missing, #00ff00)",
            ScopeKey(0),
            &env,
            &mut dropped,
        );
        assert_eq!(style.color, Color::rgb(0x00, 0xff, 0x00));
        assert!(dropped.is_empty(), "a fallback-resolved var must not drop");

        // No definition and no fallback: routes to dropped, does not apply.
        let mut style = ComputedStyle::default();
        apply_deferred_against_env(
            &mut style,
            "color",
            "var(--missing)",
            ScopeKey(3),
            &env,
            &mut dropped,
        );
        assert_eq!(dropped.len(), 1, "an unresolvable scoped var must route to dropped");
        assert_eq!(dropped[0].property, "color");
        // The bogus value did not apply.
        assert_eq!(style.color, ComputedStyle::default().color);
    }
}
