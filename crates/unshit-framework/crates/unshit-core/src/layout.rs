use crate::dirty::DirtyFlags;
use crate::element::{ElementContent, InputType, Tag};
use crate::id::NodeId;
use crate::style::types::WhiteSpace;
use crate::tree::NodeArena;
use cosmic_text::{Attrs, Buffer, FontSystem, Metrics, Shaping};
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
    font_size_tenths: i32,
    line_height_tenths: i32,
    letter_spacing_tenths: i32,
    max_width_tenths: i32, // -1 for None
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

    fn key(
        text: &str,
        font_size: f32,
        line_height: f32,
        letter_spacing: f32,
        max_width: Option<f32>,
    ) -> MeasureCacheKey {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        MeasureCacheKey {
            text_hash: hasher.finish(),
            font_size_tenths: (font_size * 10.0) as i32,
            line_height_tenths: (line_height * 10.0) as i32,
            letter_spacing_tenths: (letter_spacing * 10.0) as i32,
            max_width_tenths: max_width.map_or(-1, |w| (w * 10.0) as i32),
        }
    }

    fn get(
        &self,
        text: &str,
        font_size: f32,
        line_height: f32,
        letter_spacing: f32,
        max_width: Option<f32>,
    ) -> Option<(f32, f32)> {
        let key = Self::key(text, font_size, line_height, letter_spacing, max_width);
        self.map.get(&key).copied()
    }

    fn insert(
        &mut self,
        text: &str,
        font_size: f32,
        line_height: f32,
        letter_spacing: f32,
        max_width: Option<f32>,
        result: (f32, f32),
    ) {
        let key = Self::key(text, font_size, line_height, letter_spacing, max_width);
        self.map.insert(key, result);
    }
}

#[allow(clippy::only_used_in_recursion)]
pub fn sync_element_to_taffy(
    arena: &mut NodeArena,
    taffy: &mut TaffyTree<TextMeasureCtx>,
    node_id: NodeId,
    font_system: &mut FontSystem,
) {
    let Some(element) = arena.get(node_id) else {
        return;
    };

    let is_new = element.taffy_node.is_none();

    if is_new || element.dirty.contains(DirtyFlags::LAYOUT) {
        let mut style = element.computed_style.to_taffy_style();
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

        let measure_text = if input_needs_text_measure {
            input_text.clone()
        } else {
            match &element.content {
                ElementContent::Text(t) => t.clone(),
                _ => String::new(),
            }
        };

        if is_new {
            let taffy_node = if is_text_leaf {
                let ctx = TextMeasureCtx {
                    text: measure_text,
                    font_size: element.computed_style.font_size,
                    line_height: element.computed_style.line_height,
                    letter_spacing: element.computed_style.letter_spacing,
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
                    white_space: element.computed_style.white_space,
                };
                taffy.set_node_context(taffy_node, Some(ctx)).unwrap();
            }
        }
    }

    let child_ids = arena.children(node_id);

    for &child_id in &child_ids {
        sync_element_to_taffy(arena, taffy, child_id, font_system);
    }

    let element = arena.get(node_id).unwrap();
    if element.dirty.contains(DirtyFlags::CHILDREN) {
        let taffy_children: Vec<taffy::NodeId> =
            child_ids.iter().filter_map(|&cid| arena.get(cid).and_then(|e| e.taffy_node)).collect();
        let taffy_node = element.taffy_node.unwrap();
        taffy.set_children(taffy_node, &taffy_children).unwrap();
    }
}

/// Create a cosmic-text Buffer with text shaped and ready for layout iteration.
fn shaped_buffer(
    text: &str,
    font_size: f32,
    line_height: f32,
    max_width: Option<f32>,
    font_system: &mut FontSystem,
) -> Buffer {
    let metrics = Metrics::new(font_size, font_size * line_height);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(font_system, max_width, None);
    buffer.set_text(font_system, text, Attrs::new(), Shaping::Advanced);
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
    if let Some(ref cache) = cache {
        if let Some(cached) = cache.get(text, font_size, line_height, letter_spacing, max_width) {
            return cached;
        }
    }

    let buffer = shaped_buffer(text, font_size, line_height, max_width, font_system);

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
        cache.insert(text, font_size, line_height, letter_spacing, max_width, result);
    }

    result
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

    let buffer = shaped_buffer(text, font_size, line_height, max_width, font_system);
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

    let buffer = shaped_buffer(text, font_size, line_height, max_width, font_system);
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

    let buffer = shaped_buffer(text, font_size, line_height, max_width, font_system);
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
                    let (tw, th) = measure_text_cached(
                        &ctx.text,
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

    // If this element has text, compute distance for "nearest" fallback
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

pub fn clear_dirty_flags(arena: &mut NodeArena, node_id: NodeId) {
    if let Some(element) = arena.get_mut(node_id) {
        element.dirty = DirtyFlags::empty();
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

        sync_element_to_taffy(&mut arena, &mut taffy, parent_id, &mut font_system);
        let root_taffy = arena.get(parent_id).unwrap().taffy_node.unwrap();
        let mut cache = TextMeasureCache::new();
        compute_layout(&mut taffy, root_taffy, 800.0, 600.0, &mut font_system, &mut cache);
        read_layout_results(&mut arena, &taffy, parent_id, 0.0, 0.0);

        let svg_rect = arena.get(svg_id).unwrap().layout_rect;
        assert_eq!(svg_rect.width, 16.0, "SVG should be 16px wide from CSS");
        assert_eq!(svg_rect.height, 16.0, "SVG should be 16px tall from CSS");
    }
}
