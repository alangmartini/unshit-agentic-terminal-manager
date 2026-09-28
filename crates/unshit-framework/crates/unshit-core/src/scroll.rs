use crate::id::NodeId;
use crate::layout::TextMeasureCtx;
use crate::style::types::Overflow;
use crate::tree::NodeArena;

#[cfg(test)]
mod reveal_tests {
    use super::*;
    use crate::dirty::DirtyFlags;
    use crate::element::{Element, LayoutRect, Tag};
    use taffy::prelude::TaffyMaxContent;

    fn fixture() -> (NodeArena, taffy::TaffyTree<TextMeasureCtx>, NodeId, NodeId) {
        let mut taffy = taffy::TaffyTree::new();
        let child = taffy
            .new_leaf(taffy::Style {
                size: taffy::Size {
                    width: taffy::Dimension::Length(400.0),
                    height: taffy::Dimension::Length(500.0),
                },
                flex_shrink: 0.0,
                ..Default::default()
            })
            .unwrap();
        let parent = taffy
            .new_with_children(
                taffy::Style {
                    size: taffy::Size {
                        width: taffy::Dimension::Length(100.0),
                        height: taffy::Dimension::Length(100.0),
                    },
                    ..Default::default()
                },
                &[child],
            )
            .unwrap();
        taffy.compute_layout(parent, taffy::Size::MAX_CONTENT).unwrap();
        let mut arena = NodeArena::new();
        let mut container = Element::new(Tag::Div);
        container.taffy_node = Some(parent);
        container.layout_rect = LayoutRect { x: 10.0, y: 20.0, width: 100.0, height: 100.0 };
        container.computed_style.overflow_x = Overflow::Scroll;
        container.computed_style.overflow_y = Overflow::Scroll;
        container.dirty = DirtyFlags::empty();
        let container = arena.alloc(container);
        let mut row = Element::new(Tag::Button);
        row.layout_rect = LayoutRect { x: 160.0, y: 220.0, width: 30.0, height: 28.0 };
        row.dirty = DirtyFlags::empty();
        let row = arena.alloc(row);
        arena.append_child(container, row);
        (arena, taffy, container, row)
    }

    #[test]
    fn reveals_offscreen_row_on_both_axes_and_invalidates_paint() {
        let (mut arena, taffy, container, row) = fixture();
        assert_eq!(scroll_into_view(&mut arena, &taffy, row), Some(container));
        let element = arena.get(container).unwrap();
        assert_eq!((element.scroll_x, element.scroll_y), (80.0, 128.0));
        assert!(element.dirty.contains(DirtyFlags::PAINT));
        assert!(arena.get(row).unwrap().dirty.contains(DirtyFlags::PAINT));
        arena.get_mut(row).unwrap().layout_rect.x = 10.0;
        arena.get_mut(row).unwrap().layout_rect.y = 20.0;
        scroll_into_view(&mut arena, &taffy, row);
        let element = arena.get(container).unwrap();
        assert_eq!((element.scroll_x, element.scroll_y), (0.0, 0.0));
    }

    #[test]
    fn already_visible_row_keeps_offsets_and_clean_paint() {
        let (mut arena, taffy, container, row) = fixture();
        let element = arena.get_mut(container).unwrap();
        element.scroll_x = 100.0;
        element.scroll_y = 160.0;
        assert_eq!(scroll_into_view(&mut arena, &taffy, row), Some(container));
        let element = arena.get(container).unwrap();
        assert_eq!((element.scroll_x, element.scroll_y), (100.0, 160.0));
        assert!(!element.dirty.contains(DirtyFlags::PAINT));
        assert!(!arena.get(row).unwrap().dirty.contains(DirtyFlags::PAINT));
    }
}

// ---------------------------------------------------------------------------
// Constants (shared with renderer)
// ---------------------------------------------------------------------------

pub const SCROLLBAR_WIDTH: f32 = 12.0;
pub const SCROLLBAR_INSET: f32 = 0.0;
pub const SCROLLBAR_BUTTON_SIZE: f32 = 18.0;
pub const MIN_THUMB_SIZE: f32 = 24.0;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollbarAxis {
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollbarPart {
    Thumb,
    TrackBefore,
    TrackAfter,
    Decrement,
    Increment,
}

#[derive(Clone, Copy, Debug)]
pub struct ScrollbarGeometry {
    pub axis: ScrollbarAxis,
    /// Length of each arrow button along the scrolling axis.
    pub button_size: f32,
    pub track_x: f32,
    pub track_y: f32,
    pub track_w: f32,
    pub track_h: f32,
    pub thumb_x: f32,
    pub thumb_y: f32,
    pub thumb_w: f32,
    pub thumb_h: f32,
    pub max_scroll: f32,
    pub content_size: f32,
    pub container_size: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct ScrollbarHit {
    pub node_id: NodeId,
    pub axis: ScrollbarAxis,
    pub part: ScrollbarPart,
    pub geometry: ScrollbarGeometry,
}

#[derive(Clone, Copy, Debug)]
pub struct ScrollbarDrag {
    pub node_id: NodeId,
    pub axis: ScrollbarAxis,
    pub grab_offset: f32,
    pub geometry: ScrollbarGeometry,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollbarVisualState {
    pub hovered_node: Option<NodeId>,
    pub hovered_axis: Option<ScrollbarAxis>,
    pub dragging_node: Option<NodeId>,
    pub dragging_axis: Option<ScrollbarAxis>,
}

impl ScrollbarVisualState {
    /// Returns thumb opacity for a given scrollbar. The thumb stays visible at
    /// rest so scrollable regions advertise that more content exists.
    /// The caller builds the full `[r, g, b, a]` color.
    pub fn thumb_alpha(&self, node_id: NodeId, axis: ScrollbarAxis) -> f32 {
        if self.dragging_node == Some(node_id) && self.dragging_axis == Some(axis) {
            1.0
        } else if self.hovered_node == Some(node_id) && self.hovered_axis == Some(axis) {
            0.8
        } else {
            0.65
        }
    }

    /// Update hover tracking from an optional hit result.
    pub fn set_hover(&mut self, hit: Option<&ScrollbarHit>) {
        if let Some(h) = hit {
            self.hovered_node = Some(h.node_id);
            self.hovered_axis = Some(h.axis);
        } else {
            self.hovered_node = None;
            self.hovered_axis = None;
        }
    }

    /// Clear drag tracking (called on mouse release).
    pub fn clear_drag(&mut self) {
        self.dragging_node = None;
        self.dragging_axis = None;
    }
}

// ---------------------------------------------------------------------------
// Content extent computation (extracted from batch.rs)
// ---------------------------------------------------------------------------

/// Compute the maximum content extent (width, height) of a node's children,
/// relative to the node's own position. Returns `(content_max_x, content_max_y)`.
pub fn content_extents(arena: &NodeArena, node_id: NodeId) -> (f32, f32) {
    let Some(element) = arena.get(node_id) else {
        return (0.0, 0.0);
    };
    let rect = element.layout_rect;
    let mut content_max_x: f32 = 0.0;
    let mut content_max_y: f32 = 0.0;

    let mut scan = element.first_child;
    while !scan.is_dangling() {
        if let Some(child_elem) = arena.get(scan) {
            let child_rect = child_elem.layout_rect;
            let child_bottom = child_rect.y - rect.y + child_rect.height;
            let child_right = child_rect.x - rect.x + child_rect.width;
            content_max_y = content_max_y.max(child_bottom);
            content_max_x = content_max_x.max(child_right);
            scan = child_elem.next_sibling;
        } else {
            break;
        }
    }

    (content_max_x, content_max_y)
}

/// Compute the maximum scroll offsets for a node from its laid out content.
pub fn compute_max_scroll(
    arena: &NodeArena,
    taffy: &taffy::TaffyTree<TextMeasureCtx>,
    node_id: NodeId,
) -> (f32, f32) {
    let Some(element) = arena.get(node_id) else {
        return (0.0, 0.0);
    };

    let container_w = element.layout_rect.width;
    let container_h = element.layout_rect.height;

    let content_size = element
        .taffy_node
        .and_then(|tn| taffy.layout(tn).ok())
        .map(|layout| (layout.content_size.width, layout.content_size.height))
        .unwrap_or((0.0, 0.0));

    ((content_size.0 - container_w).max(0.0), (content_size.1 - container_h).max(0.0))
}

fn mark_scroll_paint_dirty(arena: &mut NodeArena, node_id: NodeId) {
    crate::build::mark_paint_dirty(arena, node_id);
    crate::build::mark_node_paint_dirty(arena, node_id);
}

/// Set scroll offsets and dirty cached paint if the visual position changed.
pub fn set_scroll_position(arena: &mut NodeArena, node_id: NodeId, x: f32, y: f32) -> bool {
    let changed = if let Some(element) = arena.get_mut(node_id) {
        let changed = element.scroll_x != x || element.scroll_y != y;
        if changed {
            element.scroll_x = x;
            element.scroll_y = y;
        }
        changed
    } else {
        false
    };

    if changed {
        mark_scroll_paint_dirty(arena, node_id);
    }

    changed
}

/// Reveal a laid-out descendant in its nearest scroll container with the
/// smallest offset change. Returns the container even when already visible,
/// allowing callers to cancel an animation that would move it away again.
pub fn scroll_into_view(
    arena: &mut NodeArena,
    taffy: &taffy::TaffyTree<TextMeasureCtx>,
    target: NodeId,
) -> Option<NodeId> {
    let element = arena.get(target)?;
    let rect = element.layout_rect;
    let container = find_scroll_container(arena, element.parent)?;
    let element = arena.get(container)?;
    let viewport = element.layout_rect;
    let max_scroll = compute_max_scroll(arena, taffy, container);
    fn reveal(start: f32, size: f32, offset: f32, viewport: f32, max: f32) -> f32 {
        // An oversized target cannot fit; retain its visible portion, or bring
        // its nearest edge into view when entirely outside the viewport.
        let end = start + size;
        let next = if start < offset && end > offset + viewport {
            offset
        } else if start < offset {
            start
        } else if end > offset + viewport {
            (end - viewport).min(start)
        } else {
            offset
        };
        next.clamp(0.0, max)
    }
    let x = if element.computed_style.overflow_x == Overflow::Scroll {
        reveal(rect.x - viewport.x, rect.width, element.scroll_x, viewport.width, max_scroll.0)
    } else {
        element.scroll_x
    };
    let y = if element.computed_style.overflow_y == Overflow::Scroll {
        reveal(rect.y - viewport.y, rect.height, element.scroll_y, viewport.height, max_scroll.1)
    } else {
        element.scroll_y
    };
    set_scroll_position(arena, container, x, y);
    Some(container)
}

/// Apply wheel-style deltas to a scroll container and dirty affected paint.
pub fn scroll_by(
    arena: &mut NodeArena,
    taffy: &taffy::TaffyTree<TextMeasureCtx>,
    node_id: NodeId,
    delta_x: f32,
    delta_y: f32,
) -> bool {
    let max_scroll = compute_max_scroll(arena, taffy, node_id);
    let Some(element) = arena.get(node_id) else {
        return false;
    };

    let next_x = (element.scroll_x - delta_x).clamp(0.0, max_scroll.0);
    let next_y = (element.scroll_y - delta_y).clamp(0.0, max_scroll.1);
    set_scroll_position(arena, node_id, next_x, next_y)
}

/// Remap a wheel delta for a scroll container that can only move on X.
///
/// Most mice only report a vertical wheel, so browsers translate a
/// vertical notch into horizontal movement when the target scroller has
/// no vertical overflow but does overflow horizontally. Without this a
/// horizontally scrolling strip (a tab bar, a chip row) is unreachable
/// with an ordinary wheel.
///
/// Only a pure vertical delta is remapped: a tilt wheel that already
/// reports horizontal movement, or a container that can scroll on Y,
/// keeps the original deltas. `max_scroll` is the pair returned by
/// [`compute_max_scroll`] for the same node.
pub fn remap_wheel_delta_for_axes(
    overflow_x: Overflow,
    overflow_y: Overflow,
    max_scroll: (f32, f32),
    delta: (f32, f32),
) -> (f32, f32) {
    let (delta_x, delta_y) = delta;
    if delta_x != 0.0 || delta_y == 0.0 {
        return delta;
    }
    let can_scroll_y = overflow_y == Overflow::Scroll && max_scroll.1 > 0.0;
    let can_scroll_x = overflow_x == Overflow::Scroll && max_scroll.0 > 0.0;
    if can_scroll_y || !can_scroll_x {
        return delta;
    }
    // `scroll_by` subtracts the delta, so passing the vertical delta
    // straight through on X makes wheel-down reveal later content --
    // the same direction browsers use.
    (delta_y, 0.0)
}

/// Look up a node's overflow axes and the wheel delta it should receive,
/// applying the vertical-to-horizontal remap from
/// [`remap_wheel_delta_for_axes`].
pub fn wheel_delta_for_container(
    arena: &NodeArena,
    taffy: &taffy::TaffyTree<TextMeasureCtx>,
    node_id: NodeId,
    delta: (f32, f32),
) -> (f32, f32) {
    let Some(element) = arena.get(node_id) else {
        return delta;
    };
    let overflow_x = element.computed_style.overflow_x;
    let overflow_y = element.computed_style.overflow_y;
    let max_scroll = compute_max_scroll(arena, taffy, node_id);
    remap_wheel_delta_for_axes(overflow_x, overflow_y, max_scroll, delta)
}

/// Set one scrollbar axis from drag/track interaction and dirty affected paint.
pub fn set_axis_scroll_position(
    arena: &mut NodeArena,
    node_id: NodeId,
    axis: ScrollbarAxis,
    value: f32,
) -> bool {
    let Some(element) = arena.get(node_id) else {
        return false;
    };
    let (next_x, next_y) = match axis {
        ScrollbarAxis::Vertical => (element.scroll_x, value),
        ScrollbarAxis::Horizontal => (value, element.scroll_y),
    };

    set_scroll_position(arena, node_id, next_x, next_y)
}

// ---------------------------------------------------------------------------
// Geometry computation
// ---------------------------------------------------------------------------

/// Compute scrollbar geometry for vertical and horizontal scrollbars.
/// `render_x` / `render_y` are the container's screen-space position.
/// Returns `(vertical_geometry, horizontal_geometry)`.
pub fn compute_scrollbar_geometry(
    arena: &NodeArena,
    node_id: NodeId,
    render_x: f32,
    render_y: f32,
) -> (Option<ScrollbarGeometry>, Option<ScrollbarGeometry>) {
    let Some(element) = arena.get(node_id) else {
        return (None, None);
    };

    // The vertical scrollbar is driven by `overflow-y`, the horizontal one by
    // `overflow-x`. Bail only when neither axis scrolls.
    let scroll_x = element.computed_style.overflow_x == Overflow::Scroll;
    let scroll_y = element.computed_style.overflow_y == Overflow::Scroll;
    if !scroll_x && !scroll_y {
        return (None, None);
    }

    let (content_max_x, content_max_y) = content_extents(arena, node_id);
    let container_w = element.layout_rect.width;
    let container_h = element.layout_rect.height;

    let has_vertical =
        scroll_y && content_max_y > container_h + 1.0 && container_w >= SCROLLBAR_WIDTH;
    let has_horizontal =
        scroll_x && content_max_x > container_w + 1.0 && container_h >= SCROLLBAR_WIDTH;
    let v_geom = if has_vertical {
        let max_scroll_y = content_max_y - container_h;
        let scroll_ratio = (element.scroll_y / max_scroll_y).clamp(0.0, 1.0);

        let length = (container_h - if has_horizontal { SCROLLBAR_WIDTH } else { 0.0 }).max(0.0);
        let button_size = SCROLLBAR_BUTTON_SIZE.min(length / 3.0);
        let visual_track_h = length - button_size * 2.0;
        // Size against the usable track, excluding the arrow buttons. Inflating
        // this ratio can fill the track while content still overflows.
        let thumb_h =
            (container_h / content_max_y * visual_track_h).max(MIN_THUMB_SIZE).min(visual_track_h);
        let track_h = visual_track_h;
        let thumb_y_offset = scroll_ratio * (track_h - thumb_h);

        let track_x = render_x + container_w - SCROLLBAR_WIDTH - SCROLLBAR_INSET;
        let track_y = render_y + button_size;

        Some(ScrollbarGeometry {
            axis: ScrollbarAxis::Vertical,
            button_size,
            track_x,
            track_y,
            track_w: SCROLLBAR_WIDTH,
            track_h,
            thumb_x: track_x,
            thumb_y: track_y + thumb_y_offset,
            thumb_w: SCROLLBAR_WIDTH,
            thumb_h,
            max_scroll: max_scroll_y,
            content_size: content_max_y,
            container_size: container_h,
        })
    } else {
        None
    };

    let h_geom = if has_horizontal {
        let max_scroll_x = content_max_x - container_w;
        let scroll_ratio = (element.scroll_x / max_scroll_x).clamp(0.0, 1.0);

        let length = (container_w - if has_vertical { SCROLLBAR_WIDTH } else { 0.0 }).max(0.0);
        let button_size = SCROLLBAR_BUTTON_SIZE.min(length / 3.0);
        let visual_track_w = length - button_size * 2.0;
        let thumb_w =
            (container_w / content_max_x * visual_track_w).max(MIN_THUMB_SIZE).min(visual_track_w);
        let track_w = visual_track_w;
        let thumb_x_offset = scroll_ratio * (track_w - thumb_w);

        let track_x = render_x + button_size;
        let track_y = render_y + container_h - SCROLLBAR_WIDTH - SCROLLBAR_INSET;

        Some(ScrollbarGeometry {
            axis: ScrollbarAxis::Horizontal,
            button_size,
            track_x,
            track_y,
            track_w,
            track_h: SCROLLBAR_WIDTH,
            thumb_x: track_x + thumb_x_offset,
            thumb_y: track_y,
            thumb_w,
            thumb_h: SCROLLBAR_WIDTH,
            max_scroll: max_scroll_x,
            content_size: content_max_x,
            container_size: container_w,
        })
    } else {
        None
    };

    (v_geom, h_geom)
}

// ---------------------------------------------------------------------------
// Hit testing
// ---------------------------------------------------------------------------

/// Check if a point falls within a scrollbar's track/thumb region.
pub fn scrollbar_hit_test(
    geom_v: Option<&ScrollbarGeometry>,
    geom_h: Option<&ScrollbarGeometry>,
    node_id: NodeId,
    x: f32,
    y: f32,
) -> Option<ScrollbarHit> {
    // Check vertical scrollbar first (it draws on the right edge, on top)
    if let Some(geom) = geom_v {
        if x >= geom.track_x
            && x <= geom.track_x + geom.track_w
            && y >= geom.track_y - geom.button_size
            && y <= geom.track_y + geom.track_h + geom.button_size
        {
            let part = if y < geom.track_y {
                ScrollbarPart::Decrement
            } else if y > geom.track_y + geom.track_h {
                ScrollbarPart::Increment
            } else if y >= geom.thumb_y && y <= geom.thumb_y + geom.thumb_h {
                ScrollbarPart::Thumb
            } else if y < geom.thumb_y {
                ScrollbarPart::TrackBefore
            } else {
                ScrollbarPart::TrackAfter
            };
            return Some(ScrollbarHit {
                node_id,
                axis: ScrollbarAxis::Vertical,
                part,
                geometry: *geom,
            });
        }
    }

    // Check horizontal scrollbar
    if let Some(geom) = geom_h {
        if x >= geom.track_x - geom.button_size
            && x <= geom.track_x + geom.track_w + geom.button_size
            && y >= geom.track_y
            && y <= geom.track_y + geom.track_h
        {
            let part = if x < geom.track_x {
                ScrollbarPart::Decrement
            } else if x > geom.track_x + geom.track_w {
                ScrollbarPart::Increment
            } else if x >= geom.thumb_x && x <= geom.thumb_x + geom.thumb_w {
                ScrollbarPart::Thumb
            } else if x < geom.thumb_x {
                ScrollbarPart::TrackBefore
            } else {
                ScrollbarPart::TrackAfter
            };
            return Some(ScrollbarHit {
                node_id,
                axis: ScrollbarAxis::Horizontal,
                part,
                geometry: *geom,
            });
        }
    }

    None
}

/// Walk the tree (DFS) and find the scrollbar under the cursor, if any.
/// Checks scrollbar before recursing into children (scrollbar draws on top).
pub fn find_scrollbar_at(arena: &NodeArena, root: NodeId, x: f32, y: f32) -> Option<ScrollbarHit> {
    find_scrollbar_recursive(arena, root, x, y, 0.0, 0.0)
}

fn find_scrollbar_recursive(
    arena: &NodeArena,
    node_id: NodeId,
    x: f32,
    y: f32,
    scroll_offset_x: f32,
    scroll_offset_y: f32,
) -> Option<ScrollbarHit> {
    let element = arena.get(node_id)?;
    let rect = element.layout_rect;

    let render_x = rect.x - scroll_offset_x;
    let render_y = rect.y - scroll_offset_y;

    // Check if cursor is within this node's bounds (using rendered position)
    if x < render_x || x > render_x + rect.width || y < render_y || y > render_y + rect.height {
        return None;
    }

    // If this node has overflow:scroll on either axis, check its scrollbars
    // first (they draw on top)
    if element.computed_style.overflow_x == Overflow::Scroll
        || element.computed_style.overflow_y == Overflow::Scroll
    {
        let (v_geom, h_geom) = compute_scrollbar_geometry(arena, node_id, render_x, render_y);
        if let Some(hit) = scrollbar_hit_test(v_geom.as_ref(), h_geom.as_ref(), node_id, x, y) {
            return Some(hit);
        }
    }

    // Compute child scroll offsets: when recursing into children of a scrollable
    // node, add the node's scroll offsets to the accumulated offsets.
    let child_scroll_x = scroll_offset_x + element.scroll_x;
    let child_scroll_y = scroll_offset_y + element.scroll_y;

    // Walk children in reverse order (last child = frontmost)
    let mut child = element.last_child;
    while !child.is_dangling() {
        if let Some(hit) =
            find_scrollbar_recursive(arena, child, x, y, child_scroll_x, child_scroll_y)
        {
            return Some(hit);
        }
        child = arena.get(child).map(|e| e.prev_sibling).unwrap_or(NodeId::DANGLING);
    }

    None
}

// ---------------------------------------------------------------------------
// Scroll container lookup (moved from app.rs / input.rs)
// ---------------------------------------------------------------------------

/// Walk up the parent chain from `start` looking for a node with `overflow: scroll`.
pub fn find_scroll_container(arena: &NodeArena, start: NodeId) -> Option<NodeId> {
    let mut current = start;
    while !current.is_dangling() {
        if let Some(element) = arena.get(current) {
            if element.computed_style.overflow_x == Overflow::Scroll
                || element.computed_style.overflow_y == Overflow::Scroll
            {
                return Some(current);
            }
            current = element.parent;
        } else {
            break;
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Drag / track-click scroll computation
// ---------------------------------------------------------------------------

/// Move one small step from an arrow button, preserving the other axis.
pub fn scroll_from_arrow(arena: &mut NodeArena, hit: &ScrollbarHit) -> bool {
    let Some(element) = arena.get(hit.node_id) else { return false };
    let current = match hit.axis {
        ScrollbarAxis::Vertical => element.scroll_y,
        ScrollbarAxis::Horizontal => element.scroll_x,
    };
    let step = match hit.part {
        ScrollbarPart::Decrement => -40.0,
        ScrollbarPart::Increment => 40.0,
        _ => return false,
    };
    set_axis_scroll_position(
        arena,
        hit.node_id,
        hit.axis,
        (current + step).clamp(0.0, hit.geometry.max_scroll),
    )
}

/// Given an active drag and the current cursor position on the drag axis,
/// compute the new scroll offset.
pub fn scroll_from_drag(drag: &ScrollbarDrag, cursor_pos: f32) -> f32 {
    let (track_start, track_length, thumb_length) = match drag.axis {
        ScrollbarAxis::Vertical => {
            (drag.geometry.track_y, drag.geometry.track_h, drag.geometry.thumb_h)
        }
        ScrollbarAxis::Horizontal => {
            (drag.geometry.track_x, drag.geometry.track_w, drag.geometry.thumb_w)
        }
    };

    let available = track_length - thumb_length;
    if available <= 0.0 {
        return 0.0;
    }

    let thumb_pos = cursor_pos - track_start - drag.grab_offset;
    let scroll_ratio = thumb_pos / available;
    (scroll_ratio * drag.geometry.max_scroll).clamp(0.0, drag.geometry.max_scroll)
}

/// Compute scroll offset that centers the thumb at the click position on the track.
pub fn scroll_from_track_click(geom: &ScrollbarGeometry, cursor_pos: f32) -> f32 {
    let (track_start, track_length, thumb_length) = match geom.axis {
        ScrollbarAxis::Vertical => (geom.track_y, geom.track_h, geom.thumb_h),
        ScrollbarAxis::Horizontal => (geom.track_x, geom.track_w, geom.thumb_w),
    };

    let available = track_length - thumb_length;
    if available <= 0.0 {
        return 0.0;
    }

    let thumb_pos = cursor_pos - track_start - thumb_length / 2.0;
    let scroll_ratio = thumb_pos / available;
    (scroll_ratio * geom.max_scroll).clamp(0.0, geom.max_scroll)
}

#[cfg(test)]
mod visual_state_tests {
    use super::*;

    fn node(index: u32) -> NodeId {
        NodeId { index, generation: 0 }
    }

    #[test]
    fn scrollbar_thumb_has_resting_visibility() {
        let state = ScrollbarVisualState::default();

        assert!(
            state.thumb_alpha(node(1), ScrollbarAxis::Vertical) >= 0.25,
            "scrollbar thumb should remain visible when content overflows"
        );
    }

    #[test]
    fn scrollbar_thumb_gets_stronger_while_interacting() {
        let target = node(1);
        let hovered = ScrollbarVisualState {
            hovered_node: Some(target),
            hovered_axis: Some(ScrollbarAxis::Vertical),
            dragging_node: None,
            dragging_axis: None,
        };
        let dragging = ScrollbarVisualState {
            dragging_node: Some(target),
            dragging_axis: Some(ScrollbarAxis::Vertical),
            ..hovered
        };

        let resting = ScrollbarVisualState::default().thumb_alpha(target, ScrollbarAxis::Vertical);
        let hover_alpha = hovered.thumb_alpha(target, ScrollbarAxis::Vertical);
        let drag_alpha = dragging.thumb_alpha(target, ScrollbarAxis::Vertical);

        assert!(hover_alpha > resting);
        assert!(drag_alpha > hover_alpha);
    }
}

#[cfg(test)]
mod geometry_tests {
    use super::*;
    use crate::element::{Element, LayoutRect, Tag};

    #[test]
    fn slightly_overflowing_content_has_proportional_moving_thumbs() {
        for axis in [ScrollbarAxis::Vertical, ScrollbarAxis::Horizontal] {
            let mut arena = NodeArena::new();
            let mut container = Element::new(Tag::Div);
            container.layout_rect = LayoutRect { x: 0.0, y: 0.0, width: 1000.0, height: 1000.0 };
            container.computed_style.overflow_x = Overflow::Scroll;
            container.computed_style.overflow_y = Overflow::Scroll;
            let container = arena.alloc(container);
            let mut content = Element::new(Tag::Div);
            content.layout_rect = LayoutRect { x: 0.0, y: 0.0, width: 1100.0, height: 1100.0 };
            let content = arena.alloc(content);
            arena.append_child(container, content);

            for offset in [0.0, 50.0, 100.0] {
                set_scroll_position(&mut arena, container, offset, offset);
                let (vertical, horizontal) =
                    compute_scrollbar_geometry(&arena, container, 0.0, 0.0);
                let (geom, track, thumb, position, start) = match axis {
                    ScrollbarAxis::Vertical => {
                        let g = vertical.unwrap();
                        (g, g.track_h, g.thumb_h, g.thumb_y, g.track_y)
                    }
                    ScrollbarAxis::Horizontal => {
                        let g = horizontal.unwrap();
                        (g, g.track_w, g.thumb_w, g.thumb_x, g.track_x)
                    }
                };
                assert!(thumb < track, "overflow must leave room for thumb movement: {axis:?}");
                assert!((thumb / track - 1000.0 / 1100.0).abs() < 0.0001);
                let expected = start + offset / geom.max_scroll * (track - thumb);
                assert!((position - expected).abs() < 0.001);
                let drag =
                    ScrollbarDrag { node_id: container, axis, grab_offset: 0.0, geometry: geom };
                assert!((scroll_from_drag(&drag, position) - offset).abs() < 0.001);
            }
        }
    }
}

#[cfg(test)]
mod wheel_remap_tests {
    use super::*;

    #[test]
    fn vertical_wheel_moves_horizontal_only_container() {
        // A single-row tab strip: overflow-x: auto, no vertical overflow.
        let remapped = remap_wheel_delta_for_axes(
            Overflow::Scroll,
            Overflow::Visible,
            (400.0, 0.0),
            (0.0, -120.0),
        );
        assert_eq!(
            remapped,
            (-120.0, 0.0),
            "wheel-down over a horizontal-only scroller should move it right"
        );
    }

    #[test]
    fn vertical_wheel_left_alone_when_container_scrolls_vertically() {
        // Wrapped multi-row tab strip: overflow-y: auto with real overflow.
        let delta = (0.0, -120.0);
        assert_eq!(
            remap_wheel_delta_for_axes(Overflow::Visible, Overflow::Scroll, (0.0, 76.0), delta),
            delta,
            "a vertically scrollable container keeps the vertical delta"
        );
        assert_eq!(
            remap_wheel_delta_for_axes(Overflow::Scroll, Overflow::Scroll, (400.0, 76.0), delta),
            delta,
            "a container that scrolls both ways keeps the vertical delta"
        );
    }

    #[test]
    fn wheel_remap_needs_horizontal_room() {
        let delta = (0.0, -120.0);
        assert_eq!(
            remap_wheel_delta_for_axes(Overflow::Scroll, Overflow::Visible, (0.0, 0.0), delta),
            delta,
            "no horizontal overflow means nothing to remap onto"
        );
    }

    #[test]
    fn tilt_wheel_and_idle_deltas_pass_through() {
        let tilt = (-30.0, 0.0);
        assert_eq!(
            remap_wheel_delta_for_axes(Overflow::Scroll, Overflow::Visible, (400.0, 0.0), tilt),
            tilt,
            "a horizontal delta is already usable and must not be rewritten"
        );
        let both = (-30.0, -120.0);
        assert_eq!(
            remap_wheel_delta_for_axes(Overflow::Scroll, Overflow::Visible, (400.0, 0.0), both),
            both,
            "a trackpad reporting both axes keeps its own deltas"
        );
        let idle = (0.0, 0.0);
        assert_eq!(
            remap_wheel_delta_for_axes(Overflow::Scroll, Overflow::Visible, (400.0, 0.0), idle),
            idle
        );
    }
}
