use crate::dirty::DirtyFlags;
use crate::element::{Element, ElementContent, InputType, Tag};
use crate::id::NodeId;
use crate::style::parse::PseudoElement;
use crate::style::types::{apply_text_transform, ComputedStyle, FontStyle, FontWeight, WhiteSpace};
use crate::tree::NodeArena;
use cosmic_text::{
    Attrs, Buffer, CacheKeyFlags, Family, FontSystem, Metrics, Shaping, Style, Weight,
};
use rustc_hash::FxHashMap;
use taffy::TaffyTree;

/// Context stored on taffy leaf nodes that contain text.
/// Used by the measure function during layout to compute text dimensions
/// given the available width from the parent container.
pub struct TextMeasureCtx {
    pub text: String,
    pub font_size: f32,
    pub line_height: f32,
    pub letter_spacing: f32,
    pub font_family: String,
    pub font_weight: FontWeight,
    pub font_style: FontStyle,
    pub white_space: WhiteSpace,
}

/// Cache for text measurement results to avoid redundant cosmic-text Buffer creation.
/// Keyed on (text_hash, font_size, line_height, letter_spacing, max_width) quantized to tenths of a pixel.
pub struct TextMeasureCache {
    map: FxHashMap<MeasureCacheKey, (f32, f32)>,
}

#[derive(Hash, Eq, PartialEq, Clone)]
struct MeasureCacheKey {
    text_hash: u64,
    font_family_hash: u64,
    font_weight: u16,
    font_style: FontStyle,
    font_size_tenths: i32,
    line_height_tenths: i32,
    letter_spacing_tenths: i32,
    max_width_tenths: i32, // -1 for None
}

pub fn font_weight_number(weight: FontWeight) -> u16 {
    match weight {
        FontWeight::Normal => 400,
        FontWeight::Bold => 700,
        FontWeight::W(weight) => weight,
    }
}

pub fn cosmic_font_weight(weight: FontWeight) -> Weight {
    let numeric = font_weight_number(weight);
    Weight(numeric)
}

pub fn cosmic_font_family(font_family: &str) -> Family<'_> {
    let family = normalize_font_family_token(font_family.trim());
    if font_family.contains(',') {
        let mut first_generic = None;
        for token in font_family.split(',').map(|part| normalize_font_family_token(part.trim())) {
            if let Some(generic) = generic_font_family(token) {
                if first_generic.is_none() {
                    first_generic = Some(generic);
                }
            } else if !token.is_empty() {
                return Family::Name(token);
            }
        }
        if let Some(generic) = first_generic {
            return generic;
        }
    }
    if family.is_empty() {
        Family::SansSerif
    } else if let Some(generic) = generic_font_family(family) {
        generic
    } else {
        Family::Name(family)
    }
}

fn normalize_font_family_token(token: &str) -> &str {
    let trimmed = token.trim();
    if trimmed.len() >= 2 {
        let bytes = trimmed.as_bytes();
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'\'' && last == b'\'') || (first == b'"' && last == b'"') {
            return &trimmed[1..trimmed.len() - 1];
        }
    }
    trimmed
}

fn generic_font_family(family: &str) -> Option<Family<'static>> {
    if family.eq_ignore_ascii_case("serif") {
        Some(Family::Serif)
    } else if family.eq_ignore_ascii_case("sans-serif") {
        Some(Family::SansSerif)
    } else if family.eq_ignore_ascii_case("cursive") {
        Some(Family::Cursive)
    } else if family.eq_ignore_ascii_case("fantasy") {
        Some(Family::Fantasy)
    } else if family.eq_ignore_ascii_case("monospace") {
        #[cfg(target_os = "windows")]
        {
            Some(Family::Name("Consolas"))
        }
        #[cfg(not(target_os = "windows"))]
        {
            Some(Family::Monospace)
        }
    } else {
        None
    }
}

pub fn text_attrs(font_family: &str, font_weight: FontWeight, font_style: FontStyle) -> Attrs<'_> {
    let attrs = Attrs::new()
        .family(cosmic_font_family(font_family))
        .weight(cosmic_font_weight(font_weight));
    match font_style {
        FontStyle::Normal => attrs,
        // Request a slanted face and also flag FAKE_ITALIC so the renderer's
        // render-time skew (SubpixelSwashCache transform) kicks in even when no
        // slanted face resolves for the family — cosmic-text never sets this
        // flag on its own, it only forwards what the Attrs carry.
        FontStyle::Italic | FontStyle::Oblique => {
            attrs.style(Style::Italic).cache_key_flags(CacheKeyFlags::FAKE_ITALIC)
        }
    }
}

impl Default for TextMeasureCache {
    fn default() -> Self {
        Self::new()
    }
}

impl TextMeasureCache {
    pub fn new() -> Self {
        Self { map: FxHashMap::with_capacity_and_hasher(256, Default::default()) }
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    #[allow(clippy::too_many_arguments)]
    fn key(
        text: &str,
        font_family: &str,
        font_weight: FontWeight,
        font_style: FontStyle,
        font_size: f32,
        line_height: f32,
        letter_spacing: f32,
        max_width: Option<f32>,
    ) -> MeasureCacheKey {
        use std::hash::{Hash, Hasher};
        let mut text_hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut text_hasher);
        let mut font_hasher = std::collections::hash_map::DefaultHasher::new();
        font_family.hash(&mut font_hasher);
        MeasureCacheKey {
            text_hash: text_hasher.finish(),
            font_family_hash: font_hasher.finish(),
            font_weight: font_weight_number(font_weight),
            font_style,
            font_size_tenths: (font_size * 10.0) as i32,
            line_height_tenths: (line_height * 10.0) as i32,
            letter_spacing_tenths: (letter_spacing * 10.0) as i32,
            max_width_tenths: max_width.map_or(-1, |w| (w * 10.0) as i32),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn get(
        &self,
        text: &str,
        font_family: &str,
        font_weight: FontWeight,
        font_style: FontStyle,
        font_size: f32,
        line_height: f32,
        letter_spacing: f32,
        max_width: Option<f32>,
    ) -> Option<(f32, f32)> {
        let key = Self::key(
            text,
            font_family,
            font_weight,
            font_style,
            font_size,
            line_height,
            letter_spacing,
            max_width,
        );
        self.map.get(&key).copied()
    }

    #[allow(clippy::too_many_arguments)]
    fn insert(
        &mut self,
        text: &str,
        font_family: &str,
        font_weight: FontWeight,
        font_style: FontStyle,
        font_size: f32,
        line_height: f32,
        letter_spacing: f32,
        max_width: Option<f32>,
        result: (f32, f32),
    ) {
        let key = Self::key(
            text,
            font_family,
            font_weight,
            font_style,
            font_size,
            line_height,
            letter_spacing,
            max_width,
        );
        self.map.insert(key, result);
    }
}

#[allow(clippy::only_used_in_recursion)]
pub fn sync_element_to_taffy(
    arena: &mut NodeArena,
    taffy: &mut TaffyTree<TextMeasureCtx>,
    node_id: NodeId,
    font_system: &mut FontSystem,
    viewport_w: f32,
    viewport_h: f32,
) {
    let Some(element) = arena.get(node_id) else {
        return;
    };

    let is_new = element.taffy_node.is_none();

    // Maintain the anonymous text box BEFORE the taffy sync and the child
    // snapshot below, so a child created here is registered and attached in
    // this same pass. This is the single mutation point for anonymous text
    // children: every pipeline (app rebuild/restyle/init, test harness)
    // funnels through this function, and every structural or stylistic
    // change that can alter the outcome marks the host LAYOUT or CHILDREN
    // dirty (reconcile content change: LAYOUT; pseudo add/remove:
    // CHILDREN|LAYOUT; cascade restyle: LAYOUT).
    if !element.synthetic
        && !element.anonymous
        && (is_new || element.dirty.intersects(DirtyFlags::LAYOUT | DirtyFlags::CHILDREN))
    {
        sync_anonymous_text_child(arena, taffy, node_id);
    }

    let Some(element) = arena.get(node_id) else {
        return;
    };

    if is_new || element.dirty.contains(DirtyFlags::LAYOUT) {
        let mut style = element.computed_style.to_taffy_style(viewport_w, viewport_h);
        // Hidden inputs have zero layout footprint.
        if element.tag == Tag::Input && element.input_state.input_type == InputType::Hidden {
            style.display = taffy::Display::None;
        }

        // Input elements: text-based types (text/password/number) need
        // text measurement context; others (checkbox/radio/range/hidden) do not.
        let input_needs_text_measure = element.tag == Tag::Input
            && matches!(
                element.input_state.input_type,
                InputType::Text | InputType::Password | InputType::Number
            );

        let input_text = if input_needs_text_measure {
            let v = &element.input_state.value;
            if v.is_empty() {
                element.placeholder.as_deref().unwrap_or(" ").to_string()
            } else {
                v.clone()
            }
        } else {
            String::new()
        };

        let is_text_leaf = if input_needs_text_measure {
            true
        } else {
            matches!(
                &element.content,
                ElementContent::Text(t) if !t.is_empty()
            ) && !element.has_children()
        };

        let raw_measure_text = if input_needs_text_measure {
            input_text.clone()
        } else {
            match &element.content {
                ElementContent::Text(t) => t.clone(),
                _ => String::new(),
            }
        };
        let measure_text =
            apply_text_transform(&raw_measure_text, element.computed_style.text_transform)
                .into_owned();

        if is_new {
            let taffy_node = if is_text_leaf {
                let ctx = TextMeasureCtx {
                    text: measure_text,
                    font_size: element.computed_style.font_size,
                    line_height: element.computed_style.line_height,
                    letter_spacing: element.computed_style.letter_spacing,
                    font_family: element.computed_style.font_family.clone(),
                    font_weight: element.computed_style.font_weight,
                    font_style: element.computed_style.font_style,
                    white_space: element.computed_style.white_space,
                };
                taffy.new_leaf_with_context(style, ctx).unwrap()
            } else {
                taffy.new_leaf(style).unwrap()
            };
            let element = arena.get_mut(node_id).unwrap();
            element.taffy_node = Some(taffy_node);
        } else {
            let taffy_node = element.taffy_node.unwrap();
            taffy.set_style(taffy_node, style).unwrap();
            if is_text_leaf {
                let ctx = TextMeasureCtx {
                    text: measure_text,
                    font_size: element.computed_style.font_size,
                    line_height: element.computed_style.line_height,
                    letter_spacing: element.computed_style.letter_spacing,
                    font_family: element.computed_style.font_family.clone(),
                    font_weight: element.computed_style.font_weight,
                    font_style: element.computed_style.font_style,
                    white_space: element.computed_style.white_space,
                };
                taffy.set_node_context(taffy_node, Some(ctx)).unwrap();
            } else if taffy.get_node_context(taffy_node).is_some() {
                // A former text leaf may have gained children (e.g. an
                // anonymous text box now owns the measurement); clear the
                // stale measure context. taffy ignores contexts on nodes
                // with children, but a stale context would spring back to
                // life if the node ever became a leaf again. Gated on a
                // present context: set_node_context triggers a mark_dirty
                // walk to the root, which the common container case (never
                // had a context) must not pay on every restyle frame.
                taffy.set_node_context(taffy_node, None).unwrap();
            }
        }
    }

    let child_ids = arena.children(node_id);

    for &child_id in &child_ids {
        sync_element_to_taffy(arena, taffy, child_id, font_system, viewport_w, viewport_h);
    }

    let element = arena.get(node_id).unwrap();
    if element.dirty.contains(DirtyFlags::CHILDREN) {
        let taffy_children: Vec<taffy::NodeId> =
            child_ids.iter().filter_map(|&cid| arena.get(cid).and_then(|e| e.taffy_node)).collect();
        let taffy_node = element.taffy_node.unwrap();
        taffy.set_children(taffy_node, &taffy_children).unwrap();
    }
}

/// Returns true when `element` must own an anonymous text box: it carries
/// non-empty text of its own AND has at least one child other than the
/// (possible) anonymous box itself, so taffy would never call its measure
/// function (taffy only measures childless leaves). Inputs measure their
/// own value/placeholder text and Select consumes its children into
/// `select_state`; synthetic and anonymous nodes can never be hosts.
fn wants_anon_text_child(arena: &NodeArena, element: &Element, anon: Option<NodeId>) -> bool {
    if element.synthetic
        || element.anonymous
        || matches!(element.tag, Tag::Input | Tag::Select)
        || !matches!(&element.content, ElementContent::Text(t) if !t.is_empty())
    {
        return false;
    }
    // Any child besides the anonymous box itself?
    let mut child = element.first_child;
    while !child.is_dangling() {
        if Some(child) != anon {
            return true;
        }
        child = arena.get(child).map(|e| e.next_sibling).unwrap_or(NodeId::DANGLING);
    }
    false
}

/// Create, update, or tear down the anonymous text box of `host_id` so that
/// exactly the hosts matched by [`wants_anon_text_child`] own one, its text
/// mirrors the host's content, and its style is freshly derived from the
/// host's (already cascaded and DPI-scaled) computed style. The host's
/// `computed_style` is the only style source for the anonymous child: the
/// cascade and `scale_all_styles` skip anonymous nodes, and transition /
/// animation ticks refresh the derivation through
/// [`refresh_anon_text_style`]. Only called from `sync_element_to_taffy`,
/// which immediately re-snapshots children and runs the taffy child sync —
/// calling this in isolation would leave a fresh box with no taffy node.
fn sync_anonymous_text_child(
    arena: &mut NodeArena,
    taffy: &mut TaffyTree<TextMeasureCtx>,
    host_id: NodeId,
) {
    let Some(host) = arena.get(host_id) else {
        return;
    };

    let had_handle = host.anon_text_child.is_some();
    // Validate the stored child id: reconcile can dealloc whole subtrees
    // without notifying anyone, and generational ids make stale reads safe.
    let stored = host
        .anon_text_child
        .filter(|&id| arena.get(id).map(|e| e.anonymous && e.parent == host_id).unwrap_or(false));

    let needs = wants_anon_text_child(arena, host, stored);

    match (needs, stored) {
        (false, None) => {
            if had_handle {
                // Stale handle left behind by an external teardown.
                if let Some(h) = arena.get_mut(host_id) {
                    h.anon_text_child = None;
                }
            }
        }
        (false, Some(anon_id)) => {
            remove_anon_text_child(arena, taffy, host_id, anon_id);
        }
        (true, None) => {
            create_anon_text_child(arena, host_id);
        }
        (true, Some(anon_id)) => {
            update_anon_text_child(arena, host_id, anon_id);
        }
    }
}

fn create_anon_text_child(arena: &mut NodeArena, host_id: NodeId) {
    let (text, derived) = {
        let host = arena.get(host_id).unwrap();
        let text = match &host.content {
            ElementContent::Text(t) => t.clone(),
            _ => unreachable!("wants_anon_text_child checked content"),
        };
        (text, ComputedStyle::derive_anonymous_text(&host.computed_style))
    };

    let mut elem = Element::new(Tag::Span);
    elem.parent = host_id;
    elem.content = ElementContent::Text(text);
    elem.computed_style = derived;
    elem.synthetic = true;
    elem.anonymous = true;
    // Explicit creation flags: LAYOUT so the sync registers the taffy leaf,
    // PAINT so the batch emits it, CONTENT for persistent-buffer parity
    // with the pseudo resolver. Deliberately NOT the `Element::new`
    // defaults: no STYLE (the cascade skips anonymous nodes, and a stray
    // STYLE flag would trip the app's style-work gate) and no CHILDREN
    // (text boxes are leaves).
    elem.dirty = DirtyFlags::LAYOUT | DirtyFlags::PAINT | DirtyFlags::CONTENT;
    let anon_id = arena.alloc(elem);

    link_anon_text_child(arena, host_id, anon_id);

    if let Some(host) = arena.get_mut(host_id) {
        host.anon_text_child = Some(anon_id);
        host.dirty |= DirtyFlags::CHILDREN | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
    }
    // The host's cached batch range baked its directly-painted text; make
    // sure the replay gate cannot serve it.
    crate::build::mark_node_paint_dirty(arena, host_id);
}

fn update_anon_text_child(arena: &mut NodeArena, host_id: NodeId, anon_id: NodeId) {
    // The common case on blanket-LAYOUT restyle frames is "nothing
    // changed"; compare the text by reference before allocating a clone.
    let text_changed = {
        let host_text = match arena.get(host_id).map(|h| &h.content) {
            Some(ElementContent::Text(t)) => t,
            _ => unreachable!("wants_anon_text_child checked content"),
        };
        !matches!(arena.get(anon_id).map(|a| &a.content),
            Some(ElementContent::Text(t)) if t == host_text)
    };

    let derived = ComputedStyle::derive_anonymous_text(&arena.get(host_id).unwrap().computed_style);

    let mut changed = false;
    if text_changed {
        let text = match &arena.get(host_id).unwrap().content {
            ElementContent::Text(t) => t.clone(),
            _ => unreachable!("wants_anon_text_child checked content"),
        };
        if let Some(anon) = arena.get_mut(anon_id) {
            anon.content = ElementContent::Text(text);
            anon.dirty |= DirtyFlags::LAYOUT | DirtyFlags::CONTENT | DirtyFlags::PAINT;
            changed = true;
        }
    }
    if let Some(anon) = arena.get_mut(anon_id) {
        if anon.computed_style != derived {
            anon.computed_style = derived;
            anon.dirty |= DirtyFlags::LAYOUT | DirtyFlags::PAINT;
            changed = true;
        }
    }
    if changed {
        crate::build::mark_node_paint_dirty(arena, anon_id);
    }
}

fn remove_anon_text_child(
    arena: &mut NodeArena,
    taffy: &mut TaffyTree<TextMeasureCtx>,
    host_id: NodeId,
    anon_id: NodeId,
) {
    // Free the taffy node first so no stale handle is left behind
    // (mirrors `remove_pseudo_node`).
    if let Some(elem) = arena.get(anon_id) {
        if let Some(tn) = elem.taffy_node {
            let _ = taffy.remove(tn);
        }
    }

    arena.remove_child(host_id, anon_id);
    arena.dealloc(anon_id);

    if let Some(host) = arena.get_mut(host_id) {
        host.anon_text_child = None;
        host.dirty |= DirtyFlags::CHILDREN | DirtyFlags::LAYOUT | DirtyFlags::PAINT;
    }
    // The host reverts to painting its own text; invalidate cached ranges.
    crate::build::mark_node_paint_dirty(arena, host_id);
}

/// Link a freshly allocated anonymous text box into the host's child list:
/// after any leading ::before / ::placeholder pseudo nodes and before
/// everything else (user children and ::after), matching browser anonymous
/// box order. Unknown synthetic kinds are treated as a boundary (the box is
/// inserted before them), which fails safe to "text before decoration".
fn link_anon_text_child(arena: &mut NodeArena, host_id: NodeId, anon_id: NodeId) {
    let mut cursor = arena.get(host_id).map(|h| h.first_child).unwrap_or(NodeId::DANGLING);
    while !cursor.is_dangling() {
        let Some(child) = arena.get(cursor) else {
            break;
        };
        let leading_pseudo = child.synthetic
            && matches!(
                child.pseudo_slot,
                Some(PseudoElement::Before) | Some(PseudoElement::Placeholder)
            );
        if !leading_pseudo {
            break;
        }
        cursor = child.next_sibling;
    }

    if cursor.is_dangling() {
        arena.append_child(host_id, anon_id);
    } else {
        arena.insert_child_before(host_id, anon_id, cursor);
    }
}

/// Re-derive an anonymous text box's style from its host's current computed
/// style. Called from transition/animation ticks, which mutate host styles
/// on PAINT-only frames where the layout sync (the normal derivation point)
/// never runs. Without this, an anonymous box's color/opacity/font would
/// freeze at the value captured on the last layout frame.
///
/// Paint-only by design: the box's taffy measure context intentionally
/// keeps the transition's TARGET values (written by the cascade on the
/// transition-start frame), exactly like a plain childless text leaf, so
/// final layout is correct without per-tick relayout.
///
/// Fully self-contained damage-wise: on a change it marks the box PAINT
/// and propagates SUBTREE_PAINT up the ancestor chain. Returns the box's
/// id when a change was applied (for tests/observability).
pub fn refresh_anon_text_style(arena: &mut NodeArena, host_id: NodeId) -> Option<NodeId> {
    let host = arena.get(host_id)?;
    let anon_id = host.anon_text_child?;
    if !arena.get(anon_id).map(|e| e.anonymous && e.parent == host_id).unwrap_or(false) {
        return None;
    }
    let derived = ComputedStyle::derive_anonymous_text(&arena.get(host_id)?.computed_style);
    if arena.get(anon_id)?.computed_style == derived {
        return None;
    }
    let anon = arena.get_mut(anon_id)?;
    anon.computed_style = derived;
    crate::build::mark_node_paint_dirty(arena, anon_id);
    Some(anon_id)
}

/// Create a cosmic-text Buffer with text shaped and ready for layout iteration.
#[allow(clippy::too_many_arguments)]
fn shaped_buffer(
    text: &str,
    font_family: &str,
    font_weight: FontWeight,
    font_style: FontStyle,
    font_size: f32,
    line_height: f32,
    max_width: Option<f32>,
    font_system: &mut FontSystem,
) -> Buffer {
    let metrics = Metrics::new(font_size, font_size * line_height);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(font_system, max_width, None);
    buffer.set_text(
        font_system,
        text,
        text_attrs(font_family, font_weight, font_style),
        Shaping::Advanced,
    );
    buffer.shape_until_scroll(font_system, false);
    buffer
}

pub fn measure_text(
    text: &str,
    font_size: f32,
    line_height: f32,
    letter_spacing: f32,
    max_width: Option<f32>,
    font_system: &mut FontSystem,
) -> (f32, f32) {
    measure_text_cached(text, font_size, line_height, letter_spacing, max_width, font_system, None)
}

pub fn measure_text_cached(
    text: &str,
    font_size: f32,
    line_height: f32,
    letter_spacing: f32,
    max_width: Option<f32>,
    font_system: &mut FontSystem,
    cache: Option<&mut TextMeasureCache>,
) -> (f32, f32) {
    measure_text_with_style_cached(
        text,
        "",
        FontWeight::Normal,
        FontStyle::Normal,
        font_size,
        line_height,
        letter_spacing,
        max_width,
        font_system,
        cache,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn measure_text_with_style_cached(
    text: &str,
    font_family: &str,
    font_weight: FontWeight,
    font_style: FontStyle,
    font_size: f32,
    line_height: f32,
    letter_spacing: f32,
    max_width: Option<f32>,
    font_system: &mut FontSystem,
    cache: Option<&mut TextMeasureCache>,
) -> (f32, f32) {
    if let Some(ref cache) = cache {
        if let Some(cached) = cache.get(
            text,
            font_family,
            font_weight,
            font_style,
            font_size,
            line_height,
            letter_spacing,
            max_width,
        ) {
            return cached;
        }
    }

    let buffer = shaped_buffer(
        text,
        font_family,
        font_weight,
        font_style,
        font_size,
        line_height,
        max_width,
        font_system,
    );

    let mut width = buffer.layout_runs().map(|r| r.line_w).fold(0.0f32, f32::max);

    if letter_spacing != 0.0 {
        let char_count = text.chars().count();
        if char_count > 1 {
            width += (char_count - 1) as f32 * letter_spacing;
        }
    }

    let line_count = buffer.layout_runs().count();
    let height = line_count as f32 * font_size * line_height;

    let result = (width.ceil(), height.ceil());

    if let Some(cache) = cache {
        cache.insert(
            text,
            font_family,
            font_weight,
            font_style,
            font_size,
            line_height,
            letter_spacing,
            max_width,
            result,
        );
    }

    result
}

/// The ellipsis character appended to truncated text. A single Unicode
/// horizontal-ellipsis glyph (U+2026). It is composed onto each candidate prefix
/// and the whole run is re-measured via [`painted_run_width`], so its advance is
/// always accounted for in the run's own font and shaping context.
const ELLIPSIS: &str = "\u{2026}";

/// Measure the run's PAINTED right edge for `text` in the given font, using the
/// EXACT formula the renderer paints with (mirror of `emit_text_glyphs_cached`
/// in `unshit-renderer/src/batch.rs`, the per-glyph positioning loop):
///
/// ```text
/// max over glyphs of  glyph.x + (glyph_index as f32) * letter_spacing + glyph.w
/// ```
///
/// where `glyph_index` is enumerated PER layout run (it resets at each run),
/// exactly as the renderer does via `run.glyphs.iter().enumerate()`. The text is
/// shaped single-line (`max_width = None`) so it never wraps.
///
/// This is the source of truth for "does it fit", because the clip rect cuts on
/// painted positions, not on cosmic-text's `line_w` advance metric.
#[allow(clippy::too_many_arguments)]
fn painted_run_width(
    text: &str,
    font_family: &str,
    font_weight: FontWeight,
    font_style: FontStyle,
    font_size: f32,
    line_height: f32,
    letter_spacing: f32,
    font_system: &mut FontSystem,
) -> f32 {
    let buffer = shaped_buffer(
        text,
        font_family,
        font_weight,
        font_style,
        font_size,
        line_height,
        None,
        font_system,
    );
    let mut right_edge = 0.0f32;
    for run in buffer.layout_runs() {
        for (glyph_index, glyph) in run.glyphs.iter().enumerate() {
            let edge = glyph.x + glyph_index as f32 * letter_spacing + glyph.w;
            right_edge = right_edge.max(edge);
        }
    }
    right_edge
}

/// Truncate `text` so the result PLUS an appended ellipsis fits within
/// `content_w`, cutting only on a grapheme-cluster boundary (never mid-byte,
/// never inside a combining sequence / emoji / ligature).
///
/// The fit decision is made authoritatively by measuring what is actually
/// PAINTED (via [`painted_run_width`], which mirrors the renderer's per-glyph
/// positioning formula), over LOGICAL byte prefixes of `text`. We never derive
/// a logical cut from a visual-order glyph walk: under bidi/RTL the visual order
/// is non-monotonic in source byte offset, so a visual-order cut byte can jump
/// near the end of the string and retain almost everything. Measuring the
/// composed (`prefix + ellipsis`) painted width per logical prefix is correct
/// for LTR, RTL, bidi, combining marks, ZWJ emoji, and any `letter_spacing`,
/// because it measures the exact run the renderer will paint and only ever
/// returns a valid logical prefix.
///
/// Algorithm:
///   1. Shape the source run once and collect the distinct glyph `.start` byte
///      offsets. These are valid grapheme-cluster boundaries in LOGICAL order.
///      Sort ascending and append `text.len()` so the full string is also a
///      candidate, plus `0` so the ellipsis-only result is considered.
///   2. For each boundary `b` (ascending), compose `&text[..b] + ELLIPSIS` and
///      measure its painted right edge. Keep the LARGEST `b` whose composed
///      painted width `<= content_w`. We do NOT break early on the first
///      non-fit: bidi can make composed widths slightly non-monotonic, so every
///      boundary is evaluated and the maximum fitting one is taken.
///
/// Returns `None` when no truncation is needed or possible:
///   - the full run already fits (`content_w` is wide enough — the gate should
///     prevent this, but we are defensive), or
///   - not even the ellipsis alone fits (caller falls back to clip behavior),
///   - the text is empty.
///
/// When `Some(s)` is returned, `s` is strictly a valid LOGICAL prefix of the
/// input with the ellipsis appended, and its painted width fits `content_w`.
///
/// Limitation (accepted): for RTL runs this truncates the LOGICAL tail rather
/// than the CSS-perfect VISUAL-left end. Fit is always guaranteed and the result
/// is always a valid prefix + ellipsis, which is the correctness bar here.
#[allow(clippy::too_many_arguments)]
pub fn truncate_text_with_ellipsis(
    text: &str,
    font_family: &str,
    font_weight: FontWeight,
    font_style: FontStyle,
    font_size: f32,
    line_height: f32,
    letter_spacing: f32,
    content_w: f32,
    font_system: &mut FontSystem,
) -> Option<String> {
    if text.is_empty() {
        return None;
    }

    // Defensive: if the full text already fits as painted, no truncation needed.
    // (The renderer gate already guarantees text_w > content_w before calling
    // us, but the gate measures via `line_w` advance, which can differ slightly
    // from the painted edge — re-check against the painted edge here.)
    let full_w = painted_run_width(
        text,
        font_family,
        font_weight,
        font_style,
        font_size,
        line_height,
        letter_spacing,
        font_system,
    );
    if full_w <= content_w {
        return None;
    }

    // Collect LOGICAL cluster boundaries: shape the source run once and gather
    // the distinct glyph `.start` byte offsets. Each `.start` is a
    // grapheme-cluster boundary in the source string, so slicing at one can
    // never split a multi-byte cluster (combining marks / emoji / ligature).
    let mut boundaries: Vec<usize> = {
        let buffer = shaped_buffer(
            text,
            font_family,
            font_weight,
            font_style,
            font_size,
            line_height,
            None,
            font_system,
        );
        let mut set: Vec<usize> = vec![0];
        for run in buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                set.push(glyph.start.min(text.len()));
            }
        }
        set.push(text.len());
        set.sort_unstable();
        set.dedup();
        set
    };
    // We never want to "truncate" to the full string itself (it does not fit),
    // but keeping it out of the candidate set avoids a redundant measure.
    boundaries.retain(|&b| b < text.len());

    // For each boundary (ascending), compose `prefix + ellipsis` and measure its
    // PAINTED right edge. Keep the largest boundary whose composed run fits.
    // Do NOT break early on the first non-fit: bidi can make composed widths
    // slightly non-monotonic in `b`, so evaluate all boundaries.
    let mut best: Option<String> = None;
    for &b in &boundaries {
        let composed = format!("{}{ELLIPSIS}", &text[..b]);
        let composed_w = painted_run_width(
            &composed,
            font_family,
            font_weight,
            font_style,
            font_size,
            line_height,
            letter_spacing,
            font_system,
        );
        if composed_w <= content_w {
            best = Some(composed);
        }
    }

    // `best` is None only when even the ellipsis-only composed run (b == 0) does
    // not fit, i.e. content_w is narrower than the ellipsis itself; the caller
    // then falls back to a hard clip.
    best
}

/// Pixel bounds of a single glyph cluster within a text layout.
#[derive(Clone, Debug)]
pub struct GlyphRange {
    pub byte_start: usize,
    pub byte_end: usize,
    pub x: f32,
    pub width: f32,
    pub y: f32,
    pub height: f32,
}

#[allow(clippy::too_many_arguments)]
/// Map a local (x, y) coordinate (relative to text origin) to a byte offset
/// in the text string. Returns the byte offset of the character at that position,
/// or None if the position is outside the text bounds.
///
/// `local_x` and `local_y` are relative to the top-left of the text content area
/// (after padding has been subtracted).
pub fn hit_test_text_position(
    text: &str,
    font_size: f32,
    line_height: f32,
    letter_spacing: f32,
    max_width: Option<f32>,
    local_x: f32,
    local_y: f32,
    font_system: &mut FontSystem,
) -> Option<usize> {
    if text.is_empty() {
        return None;
    }

    let buffer = shaped_buffer(
        text,
        "",
        FontWeight::Normal,
        FontStyle::Normal,
        font_size,
        line_height,
        max_width,
        font_system,
    );
    let line_h = font_size * line_height;

    let mut best_offset: Option<usize> = None;
    let mut best_distance = f32::MAX;

    for run in buffer.layout_runs() {
        let run_top = run.line_y - font_size;
        let run_bottom = run.line_y + (line_h - font_size);

        let y_in_run = local_y >= run_top - line_h * 0.5 && local_y <= run_bottom + line_h * 0.5;

        if !y_in_run {
            continue;
        }

        for (glyph_idx, glyph) in run.glyphs.iter().enumerate() {
            let ls_offset = glyph_idx as f32 * letter_spacing;
            let glyph_x = glyph.x + ls_offset;
            let glyph_center_x = glyph_x + glyph.w * 0.5;

            let dist = (local_x - glyph_center_x).abs();

            if dist < best_distance {
                best_distance = dist;
                if local_x < glyph_center_x {
                    best_offset = Some(glyph.start);
                } else {
                    best_offset = Some(glyph.end);
                }
            }
        }

        if best_offset.is_some() {
            break;
        }
    }

    best_offset.or(Some(0))
}

/// Get the x-position and width of each glyph cluster in a text string for
/// selection rendering. Returns a Vec of `GlyphRange`, one per glyph cluster.
/// Positions are relative to the text origin.
pub fn text_glyph_ranges(
    text: &str,
    font_size: f32,
    line_height: f32,
    letter_spacing: f32,
    max_width: Option<f32>,
    font_system: &mut FontSystem,
) -> Vec<GlyphRange> {
    let mut ranges = Vec::new();

    if text.is_empty() {
        return ranges;
    }

    let buffer = shaped_buffer(
        text,
        "",
        FontWeight::Normal,
        FontStyle::Normal,
        font_size,
        line_height,
        max_width,
        font_system,
    );
    let line_h = font_size * line_height;

    for run in buffer.layout_runs() {
        for (glyph_idx, glyph) in run.glyphs.iter().enumerate() {
            let ls_offset = glyph_idx as f32 * letter_spacing;
            ranges.push(GlyphRange {
                byte_start: glyph.start,
                byte_end: glyph.end,
                x: glyph.x + ls_offset,
                width: glyph.w,
                y: run.line_y - font_size,
                height: line_h,
            });
        }
    }

    ranges
}

/// Merged selection rectangle spanning an entire line of selected text.
#[derive(Clone, Debug)]
pub struct LineSelectionRange {
    pub x: f32,
    pub width: f32,
    pub y: f32,
    pub height: f32,
}

#[allow(clippy::too_many_arguments)]
/// Get merged per-line selection rectangles for a byte range in text.
/// Unlike `text_glyph_ranges()` which returns one rect per glyph, this merges
/// all selected glyphs on the same layout run into a single rectangle,
/// eliminating sub-pixel gaps between adjacent glyph quads.
pub fn text_line_ranges(
    text: &str,
    font_size: f32,
    line_height: f32,
    letter_spacing: f32,
    max_width: Option<f32>,
    sel_start: usize,
    sel_end: usize,
    font_system: &mut FontSystem,
) -> Vec<LineSelectionRange> {
    let mut result = Vec::new();

    if text.is_empty() || sel_start >= sel_end {
        return result;
    }

    let buffer = shaped_buffer(
        text,
        "",
        FontWeight::Normal,
        FontStyle::Normal,
        font_size,
        line_height,
        max_width,
        font_system,
    );
    let line_h = font_size * line_height;

    for run in buffer.layout_runs() {
        let mut min_x = f32::MAX;
        let mut max_x_plus_w = f32::MIN;

        for (glyph_idx, glyph) in run.glyphs.iter().enumerate() {
            let ls_offset = glyph_idx as f32 * letter_spacing;

            if glyph.end > sel_start && glyph.start < sel_end {
                min_x = min_x.min(glyph.x + ls_offset);
                max_x_plus_w = max_x_plus_w.max(glyph.x + ls_offset + glyph.w);
            }
        }

        if min_x < f32::MAX {
            result.push(LineSelectionRange {
                x: min_x,
                width: max_x_plus_w - min_x,
                y: run.line_y - font_size,
                height: line_h,
            });
        }
    }

    result
}

pub fn compute_layout(
    taffy: &mut TaffyTree<TextMeasureCtx>,
    root_taffy: taffy::NodeId,
    width: f32,
    height: f32,
    font_system: &mut FontSystem,
    cache: &mut TextMeasureCache,
) {
    taffy
        .compute_layout_with_measure(
            root_taffy,
            taffy::Size {
                width: taffy::AvailableSpace::Definite(width),
                height: taffy::AvailableSpace::Definite(height),
            },
            |known_dimensions, available_space, _node_id, context, _style| {
                if let Some(ctx) = context {
                    let max_width =
                        if matches!(ctx.white_space, WhiteSpace::Nowrap | WhiteSpace::Pre) {
                            // nowrap/pre: text must never wrap, so always measure
                            // without a width constraint. This ensures single-line
                            // height regardless of how narrow the container is.
                            None
                        } else {
                            known_dimensions.width.or(match available_space.width {
                                taffy::AvailableSpace::Definite(w) => Some(w),
                                _ => None,
                            })
                        };
                    let (tw, th) = measure_text_with_style_cached(
                        &ctx.text,
                        &ctx.font_family,
                        ctx.font_weight,
                        ctx.font_style,
                        ctx.font_size,
                        ctx.line_height,
                        ctx.letter_spacing,
                        max_width,
                        font_system,
                        Some(cache),
                    );
                    taffy::Size {
                        width: known_dimensions.width.unwrap_or(tw),
                        height: known_dimensions.height.unwrap_or(th),
                    }
                } else {
                    taffy::Size::ZERO
                }
            },
        )
        .unwrap();
}

pub fn read_layout_results(
    arena: &mut NodeArena,
    taffy: &TaffyTree<TextMeasureCtx>,
    node_id: NodeId,
    parent_x: f32,
    parent_y: f32,
) {
    let Some(taffy_node) = arena.get(node_id).and_then(|e| e.taffy_node) else {
        return;
    };

    let layout = taffy.layout(taffy_node).unwrap();
    let abs_x = parent_x + layout.location.x;
    let abs_y = parent_y + layout.location.y;

    let child_ids = arena.children(node_id);

    let element = arena.get_mut(node_id).unwrap();
    element.layout_rect.x = abs_x;
    element.layout_rect.y = abs_y;
    element.layout_rect.width = layout.size.width;
    element.layout_rect.height = layout.size.height;

    for &child_id in &child_ids {
        read_layout_results(arena, taffy, child_id, abs_x, abs_y);
    }
}

/// Look up the element at `node_id`, check if it contains text, convert the
/// absolute cursor coordinates to content-local coordinates, and return the
/// byte offset in the text. Returns `None` if the element is missing, has no
/// text, or the hit test fails.
pub fn text_hit_at(
    arena: &NodeArena,
    node_id: NodeId,
    cursor_x: f32,
    cursor_y: f32,
    font_system: &mut FontSystem,
) -> Option<(NodeId, usize)> {
    let element = arena.get(node_id)?;
    // When the host's text lives in an anonymous text box, the box owns the
    // hit math (the host's rect includes pseudo/user children and would
    // yield wrong wrap widths and offsets) and the returned NodeId, so text
    // selection anchors to the node whose text arm paints the highlight.
    if let Some(anon_id) = element.anon_text_child {
        if arena.get(anon_id).map(|e| e.anonymous).unwrap_or(false) {
            return text_hit_at(arena, anon_id, cursor_x, cursor_y, font_system);
        }
    }
    let text = match &element.content {
        ElementContent::Text(t) if !t.is_empty() => t,
        _ => return None,
    };
    let style = &element.computed_style;
    let rect = element.layout_rect;

    let local_x = cursor_x - rect.x - style.padding.left;
    let local_y = cursor_y - rect.y - style.padding.top;
    let content_w = rect.width - style.padding.left - style.padding.right;

    let offset = hit_test_text_position(
        text,
        style.font_size,
        style.line_height,
        style.letter_spacing,
        Some(content_w),
        local_x,
        local_y,
        font_system,
    )?;

    Some((node_id, offset))
}

/// Find the nearest text element to the cursor position.
/// Falls back by walking up the tree and searching siblings when the direct
/// hit target has no text (e.g. a button element).
pub fn nearest_text_hit_at(
    arena: &NodeArena,
    root: NodeId,
    cursor_x: f32,
    cursor_y: f32,
    font_system: &mut FontSystem,
) -> Option<(NodeId, usize)> {
    // First try direct hit test
    let hit_node = crate::event::hit_test(arena, root, cursor_x, cursor_y)?;

    // Try text hit on the hit node itself
    if let Some(result) = text_hit_at(arena, hit_node, cursor_x, cursor_y, font_system) {
        return Some(result);
    }

    // Walk up to find a container, then search its text-bearing descendants
    let mut current = hit_node;
    while !current.is_dangling() {
        if let Some(element) = arena.get(current) {
            let mut child = element.first_child;
            let mut best: Option<(NodeId, usize, f32)> = None;

            while !child.is_dangling() {
                if let result @ Some(_) = find_nearest_text_in_subtree(
                    arena,
                    child,
                    cursor_x,
                    cursor_y,
                    font_system,
                    &mut best,
                ) {
                    return result;
                }
                child = arena.get(child).map(|e| e.next_sibling).unwrap_or(NodeId::DANGLING);
            }

            if let Some((best_node, best_offset, _)) = best {
                return Some((best_node, best_offset));
            }

            current = element.parent;
        } else {
            break;
        }
    }

    None
}

fn find_nearest_text_in_subtree(
    arena: &NodeArena,
    node_id: NodeId,
    cursor_x: f32,
    cursor_y: f32,
    font_system: &mut FontSystem,
    best: &mut Option<(NodeId, usize, f32)>,
) -> Option<(NodeId, usize)> {
    let element = arena.get(node_id)?;

    // Try direct text hit
    if let Some(result) = text_hit_at(arena, node_id, cursor_x, cursor_y, font_system) {
        return Some(result);
    }

    // If this element has text, compute distance for "nearest" fallback.
    // Hosts whose text lives in an anonymous box are skipped: the box is a
    // child of this subtree and provides the candidate with correct rect
    // math and the text-owning NodeId.
    if element.anon_text_child.is_none() {
        if let ElementContent::Text(ref text) = element.content {
            if !text.is_empty() {
                let rect = element.layout_rect;
                let center_y = rect.y + rect.height * 0.5;
                let dist = (cursor_y - center_y).abs();

                let is_closer = best.as_ref().is_none_or(|(_, _, d)| dist < *d);
                if is_closer {
                    let clamped_x = cursor_x.clamp(rect.x, rect.x + rect.width);
                    let clamped_y = cursor_y.clamp(rect.y, rect.y + rect.height);
                    if let Some((_, offset)) =
                        text_hit_at(arena, node_id, clamped_x, clamped_y, font_system)
                    {
                        *best = Some((node_id, offset, dist));
                    }
                }
            }
        }
    }

    // Recurse into children
    let mut child = element.first_child;
    while !child.is_dangling() {
        if let result @ Some(_) =
            find_nearest_text_in_subtree(arena, child, cursor_x, cursor_y, font_system, best)
        {
            return result;
        }
        child = arena.get(child).map(|e| e.next_sibling).unwrap_or(NodeId::DANGLING);
    }

    None
}

/// Layout-phase flags that `clear_dirty_flags` removes. PAINT and
/// SUBTREE_PAINT are intentionally preserved so the batch builder can
/// use them to decide which nodes need re-rendering.
const LAYOUT_PHASE_FLAGS: DirtyFlags = DirtyFlags::from_bits_truncate(
    DirtyFlags::STYLE.bits()
        | DirtyFlags::LAYOUT.bits()
        | DirtyFlags::CHILDREN.bits()
        | DirtyFlags::CONTENT.bits()
        | DirtyFlags::SUBTREE_STYLE.bits()
        | DirtyFlags::SUBTREE_LAYOUT.bits(),
);

pub fn clear_dirty_flags(arena: &mut NodeArena, node_id: NodeId) {
    if let Some(element) = arena.get_mut(node_id) {
        element.dirty.remove(LAYOUT_PHASE_FLAGS);
    }

    let child_ids = arena.children(node_id);
    for &child_id in &child_ids {
        clear_dirty_flags(arena, child_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{Element, ElementDef};
    use crate::svg::types::{SvgAttrs, SvgNode, SvgPrimitive, ViewBox};

    #[test]
    fn cosmic_font_family_strips_css_quotes() {
        assert!(matches!(cosmic_font_family("'Consolas'"), Family::Name("Consolas")));
    }

    #[test]
    fn cosmic_font_family_prefers_first_concrete_family_in_css_list() {
        let family = "'JetBrains Mono', 'Berkeley Mono', 'SF Mono', Menlo, Consolas, monospace";
        assert!(matches!(cosmic_font_family(family), Family::Name("JetBrains Mono")));
    }

    #[test]
    fn cosmic_font_family_uses_generic_when_list_has_no_concrete_family() {
        let family = "monospace";
        #[cfg(target_os = "windows")]
        assert!(matches!(cosmic_font_family(family), Family::Name("Consolas")));
        #[cfg(not(target_os = "windows"))]
        assert!(matches!(cosmic_font_family(family), Family::Monospace));
    }

    #[test]
    fn cosmic_font_weight_preserves_medium_weight() {
        assert_eq!(cosmic_font_weight(FontWeight::W(500)), Weight(500));
    }

    /// SVG elements with CSS-assigned pixel dimensions (from a `svg`
    /// tag rule) must receive non-zero layout size. Without the CSS
    /// rule, auto-dimensioned SVG leaves collapse to 0x0 in taffy.
    #[test]
    fn svg_element_with_css_size_gets_nonzero_layout() {
        let mut arena = NodeArena::new();
        let mut taffy = TaffyTree::new();
        let mut font_system = FontSystem::new();

        // Parent container with explicit 26x26 size.
        let parent_def = ElementDef::new(Tag::Div);
        let parent_id = arena.alloc(Element::new(Tag::Div));
        arena.get_mut(parent_id).unwrap().update_from_def(&parent_def);
        arena.get_mut(parent_id).unwrap().computed_style.width =
            crate::style::types::Dimension::Px(26.0);
        arena.get_mut(parent_id).unwrap().computed_style.height =
            crate::style::types::Dimension::Px(26.0);

        // SVG child with CSS width/height set (simulates `svg { width: 16px; height: 16px; }`).
        let svg_node = SvgNode {
            primitive: SvgPrimitive::Group,
            attrs: SvgAttrs {
                view_box: Some(ViewBox::new(0.0, 0.0, 16.0, 16.0)),
                ..Default::default()
            },
            children: Vec::new(),
        };
        let svg_def = ElementDef::new(Tag::Div).with_svg(svg_node);
        let svg_id = arena.alloc(Element::new(Tag::Svg));
        arena.get_mut(svg_id).unwrap().update_from_def(&svg_def);
        // The CSS cascade applies `svg { width: 16px; height: 16px; }`.
        arena.get_mut(svg_id).unwrap().computed_style.width =
            crate::style::types::Dimension::Px(16.0);
        arena.get_mut(svg_id).unwrap().computed_style.height =
            crate::style::types::Dimension::Px(16.0);
        arena.append_child(parent_id, svg_id);

        sync_element_to_taffy(&mut arena, &mut taffy, parent_id, &mut font_system, 800.0, 600.0);
        let root_taffy = arena.get(parent_id).unwrap().taffy_node.unwrap();
        let mut cache = TextMeasureCache::new();
        compute_layout(&mut taffy, root_taffy, 800.0, 600.0, &mut font_system, &mut cache);
        read_layout_results(&mut arena, &taffy, parent_id, 0.0, 0.0);

        let svg_rect = arena.get(svg_id).unwrap().layout_rect;
        assert_eq!(svg_rect.width, 16.0, "SVG should be 16px wide from CSS");
        assert_eq!(svg_rect.height, 16.0, "SVG should be 16px tall from CSS");
    }

    #[test]
    fn text_measure_cache_key_includes_font_family_and_weight() {
        let mut font_system = FontSystem::new();
        let mut cache = TextMeasureCache::new();

        let _ = measure_text_with_style_cached(
            "keep",
            "Consolas",
            FontWeight::Normal,
            FontStyle::Normal,
            11.0,
            1.4,
            0.0,
            None,
            &mut font_system,
            Some(&mut cache),
        );
        let _ = measure_text_with_style_cached(
            "keep",
            "Consolas",
            FontWeight::W(600),
            FontStyle::Normal,
            11.0,
            1.4,
            0.0,
            None,
            &mut font_system,
            Some(&mut cache),
        );
        let _ = measure_text_with_style_cached(
            "keep",
            "JetBrains Mono",
            FontWeight::Normal,
            FontStyle::Normal,
            11.0,
            1.4,
            0.0,
            None,
            &mut font_system,
            Some(&mut cache),
        );

        assert_eq!(cache.len(), 3);

        let _ = measure_text_with_style_cached(
            "keep",
            "Consolas",
            FontWeight::Normal,
            FontStyle::Normal,
            11.0,
            1.4,
            0.0,
            None,
            &mut font_system,
            Some(&mut cache),
        );

        assert_eq!(cache.len(), 3);
    }

    #[test]
    fn text_measure_cache_key_distinguishes_font_style() {
        // Italic must shape/measure separately from upright so the cache never
        // serves upright metrics (or upright glyph keys) for slanted text.
        let upright = TextMeasureCache::key(
            "keep",
            "Consolas",
            FontWeight::Normal,
            FontStyle::Normal,
            11.0,
            1.4,
            0.0,
            None,
        );
        let italic = TextMeasureCache::key(
            "keep",
            "Consolas",
            FontWeight::Normal,
            FontStyle::Italic,
            11.0,
            1.4,
            0.0,
            None,
        );
        assert!(upright != italic, "italic must hash to a distinct measure cache key");
    }

    #[test]
    fn text_attrs_flags_fake_italic_for_slanted_styles() {
        // The renderer's render-time skew is gated on CacheKeyFlags::FAKE_ITALIC,
        // which cosmic-text only forwards from the Attrs — so the text path must
        // set it for italic/oblique.
        assert_eq!(
            text_attrs("Consolas", FontWeight::Normal, FontStyle::Normal).cache_key_flags,
            CacheKeyFlags::empty(),
        );
        assert_eq!(
            text_attrs("Consolas", FontWeight::Normal, FontStyle::Italic).cache_key_flags,
            CacheKeyFlags::FAKE_ITALIC,
        );
        assert_eq!(
            text_attrs("Consolas", FontWeight::Normal, FontStyle::Oblique).cache_key_flags,
            CacheKeyFlags::FAKE_ITALIC,
        );
    }

    /// Regression: clear_dirty_flags must only clear layout-phase flags
    /// (STYLE, LAYOUT, CHILDREN, CONTENT, SUBTREE_STYLE, SUBTREE_LAYOUT).
    /// PAINT and SUBTREE_PAINT must survive so the batch builder can use
    /// them to decide which nodes need re-rendering.
    #[test]
    fn clear_dirty_flags_preserves_paint_flags() {
        let mut arena = NodeArena::new();
        let parent_id = arena.alloc(Element::new(Tag::Div));
        let child_id = arena.alloc(Element::new(Tag::Div));
        arena.append_child(parent_id, child_id);

        // Set all flags on both nodes.
        let all_flags = DirtyFlags::STYLE
            | DirtyFlags::LAYOUT
            | DirtyFlags::CHILDREN
            | DirtyFlags::CONTENT
            | DirtyFlags::PAINT
            | DirtyFlags::SUBTREE_STYLE
            | DirtyFlags::SUBTREE_LAYOUT
            | DirtyFlags::SUBTREE_PAINT;
        arena.get_mut(parent_id).unwrap().dirty = all_flags;
        arena.get_mut(child_id).unwrap().dirty = all_flags;

        clear_dirty_flags(&mut arena, parent_id);

        let parent_dirty = arena.get(parent_id).unwrap().dirty;
        let child_dirty = arena.get(child_id).unwrap().dirty;

        // Layout-phase flags should be cleared.
        assert!(!parent_dirty.contains(DirtyFlags::STYLE));
        assert!(!parent_dirty.contains(DirtyFlags::LAYOUT));
        assert!(!parent_dirty.contains(DirtyFlags::CHILDREN));
        assert!(!parent_dirty.contains(DirtyFlags::CONTENT));
        assert!(!parent_dirty.contains(DirtyFlags::SUBTREE_STYLE));
        assert!(!parent_dirty.contains(DirtyFlags::SUBTREE_LAYOUT));

        // PAINT flags must survive for the batch builder.
        assert!(
            parent_dirty.contains(DirtyFlags::PAINT),
            "PAINT must survive clear_dirty_flags; got {:?}",
            parent_dirty
        );
        assert!(
            parent_dirty.contains(DirtyFlags::SUBTREE_PAINT),
            "SUBTREE_PAINT must survive clear_dirty_flags; got {:?}",
            parent_dirty
        );
        assert!(
            child_dirty.contains(DirtyFlags::PAINT),
            "child PAINT must survive; got {:?}",
            child_dirty
        );
    }

    // ---- text-overflow: ellipsis truncation -------------------------------

    /// The horizontal-ellipsis char the truncation helper appends.
    const TEST_ELLIPSIS: &str = "\u{2026}";

    /// Convenience: unconstrained single-line advance of `text` in the test
    /// font (cosmic-text's `line_w` advance metric). Used only to size content
    /// widths relative to a string's natural width.
    fn run_w(text: &str, font_system: &mut FontSystem) -> f32 {
        measure_text_with_style_cached(
            text,
            "Consolas",
            FontWeight::Normal,
            FontStyle::Normal,
            14.0,
            1.2,
            0.0,
            None,
            font_system,
            None,
        )
        .0
    }

    /// INDEPENDENT painted right-edge computation for `text` at `letter_spacing`,
    /// re-deriving the renderer formula inline so the matrix below never calls
    /// the implementation's own `painted_run_width` (avoids a circular check).
    ///
    /// Mirrors `emit_text_glyphs_cached` in `unshit-renderer/src/batch.rs`:
    /// `max over glyphs of glyph.x + glyph_index*letter_spacing + glyph.w`, with
    /// `glyph_index` enumerated PER layout run.
    fn independent_painted_edge(
        text: &str,
        font_size: f32,
        line_height: f32,
        letter_spacing: f32,
        font_system: &mut FontSystem,
    ) -> f32 {
        let metrics = Metrics::new(font_size, font_size * line_height);
        let mut buffer = Buffer::new(font_system, metrics);
        buffer.set_size(font_system, None, None);
        buffer.set_text(
            font_system,
            text,
            text_attrs("Consolas", FontWeight::Normal, FontStyle::Normal),
            Shaping::Advanced,
        );
        buffer.shape_until_scroll(font_system, false);
        let mut edge = 0.0f32;
        for run in buffer.layout_runs() {
            for (glyph_index, glyph) in run.glyphs.iter().enumerate() {
                edge = edge.max(glyph.x + glyph_index as f32 * letter_spacing + glyph.w);
            }
        }
        edge
    }

    /// THE MATRIX. For a cross product of {LTR ASCII, pure RTL Arabic, bidi
    /// mixed, combining-mark string} x {letter_spacing 0,2,6} x several content
    /// widths, assert the FIT INVARIANT using the INDEPENDENT painted-edge
    /// computation above:
    ///   - the painted right edge of the returned composed run is <=
    ///     content_w + 0.5 (the clip rect cuts on painted positions), and
    ///   - the returned text minus the trailing ellipsis is a byte-exact LOGICAL
    ///     prefix of the input.
    ///
    /// These cells genuinely fail against the OLD visual-order algorithm: the
    /// bidi cell's visual-order cut byte jumps near the end (retaining almost
    /// everything, overflowing content_w by several x), and the
    /// combining-mark + letter_spacing cells mis-reserve per-glyph spacing the
    /// renderer paints per-glyph-index, so the composed ellipsis lands past the
    /// clip rect.
    #[test]
    fn ellipsis_fit_invariant_matrix() {
        let mut font_system = FontSystem::new();

        let inputs: [&str; 4] = [
            // pure-LTR long ASCII
            "The quick brown fox jumps over the lazy dog 1234567890",
            // pure-RTL Arabic
            "مرحبا بالعالم هذا اختبار طويل",
            // bidi mixed (Arabic + Latin filename): visual order is
            // non-monotonic in source byte offset.
            "مرحبا-document-final-version-2026.txt",
            // combining marks: base 'a'/'b'/'c' each + combining acute (U+0301),
            // so char_count != glyph_count.
            "a\u{301}b\u{301}c\u{301}a\u{301}b\u{301}c\u{301}a\u{301}b\u{301}c\u{301}a\u{301}b\u{301}c\u{301}",
        ];

        for text in inputs {
            for letter_spacing in [0.0f32, 2.0, 6.0] {
                // Natural painted width at this letter_spacing, so content
                // fractions land cuts at varied boundaries.
                let natural =
                    independent_painted_edge(text, 14.0, 1.2, letter_spacing, &mut font_system);
                for frac in [0.2f32, 0.35, 0.5, 0.65, 0.8] {
                    let content_w = natural * frac;
                    let Some(result) = truncate_text_with_ellipsis(
                        text,
                        "Consolas",
                        FontWeight::Normal,
                        FontStyle::Normal,
                        14.0,
                        1.2,
                        letter_spacing,
                        content_w,
                        &mut font_system,
                    ) else {
                        // None means even the ellipsis alone did not fit; that is
                        // a valid fall-back-to-clip outcome, nothing to assert.
                        continue;
                    };

                    // Invariant 1: painted right edge fits content_w (epsilon).
                    let painted = independent_painted_edge(
                        &result,
                        14.0,
                        1.2,
                        letter_spacing,
                        &mut font_system,
                    );
                    assert!(
                        painted <= content_w + 0.5,
                        "painted edge {painted} must fit content_w {content_w} (eps 0.5) for \
                         text={text:?} ls={letter_spacing} frac={frac} result={result:?}"
                    );

                    // Invariant 2: returned text minus ellipsis is a byte-exact
                    // logical prefix of the input.
                    assert!(
                        result.ends_with(TEST_ELLIPSIS),
                        "result must end in the ellipsis; got {result:?}"
                    );
                    let retained = result.strip_suffix(TEST_ELLIPSIS).unwrap();
                    assert!(
                        text.starts_with(retained),
                        "retained {retained:?} must be a byte-exact LOGICAL prefix of \
                         {text:?} (ls={letter_spacing} frac={frac})"
                    );
                }
            }
        }
    }

    /// When `content_w` is narrower than the ellipsis itself, no prefix (not
    /// even the empty one) can fit, so the helper returns None and the caller
    /// falls back to a hard clip.
    #[test]
    fn ellipsis_returns_none_when_even_ellipsis_does_not_fit() {
        let mut font_system = FontSystem::new();
        let text = "The quick brown fox";
        let ellipsis_w = independent_painted_edge(TEST_ELLIPSIS, 14.0, 1.2, 0.0, &mut font_system);
        // Strictly narrower than the ellipsis glyph itself.
        let content_w = ellipsis_w * 0.5;
        let result = truncate_text_with_ellipsis(
            text,
            "Consolas",
            FontWeight::Normal,
            FontStyle::Normal,
            14.0,
            1.2,
            0.0,
            content_w,
            &mut font_system,
        );
        assert!(
            result.is_none(),
            "content_w narrower than the ellipsis must return None (hard-clip fallback); \
             got {result:?}"
        );
    }

    /// (d) A run that already fits is returned unchanged (None) with NO ellipsis.
    #[test]
    fn ellipsis_noop_when_text_already_fits() {
        let mut font_system = FontSystem::new();
        let text = "short";
        let full_w = run_w(text, &mut font_system);
        // Give generous room so the full run plus reserved ellipsis budget fits.
        let content_w = full_w * 4.0 + 100.0;
        let result = truncate_text_with_ellipsis(
            text,
            "Consolas",
            FontWeight::Normal,
            FontStyle::Normal,
            14.0,
            1.2,
            0.0,
            content_w,
            &mut font_system,
        );
        assert!(
            result.is_none(),
            "a run that already fits must return None (no truncation, no ellipsis); got {result:?}"
        );
    }

    /// (e) Regression guard for the Clip default: `text-overflow: clip` is the
    /// initial value and the renderer only calls the truncation helper for
    /// `Ellipsis`. This asserts the helper is purely additive — the Clip path
    /// never invokes it, so the default `ComputedStyle` carries `Clip` and the
    /// helper is not reachable for clip styles.
    #[test]
    fn text_overflow_default_is_clip_noop() {
        use crate::style::types::{ComputedStyle, TextOverflow};
        let style = ComputedStyle::default();
        assert_eq!(
            style.text_overflow,
            TextOverflow::Clip,
            "default text-overflow must be Clip so existing hard-clip behavior is unchanged"
        );
        // text-overflow does not inherit: a child must not pick up a parent's
        // Ellipsis. Build a parent with Ellipsis and confirm inherit_from leaves
        // the child at its own (default Clip) value.
        let mut parent = ComputedStyle::default();
        parent.text_overflow = TextOverflow::Ellipsis;
        let mut child = ComputedStyle::default();
        child.inherit_from(&parent);
        assert_eq!(
            child.text_overflow,
            TextOverflow::Clip,
            "text-overflow must NOT inherit from the parent"
        );
    }
}
