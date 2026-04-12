use crate::id::NodeId;
use crate::style::types::Overflow;
use crate::tree::NodeArena;

// ---------------------------------------------------------------------------
// Constants (shared with renderer)
// ---------------------------------------------------------------------------

pub const SCROLLBAR_WIDTH: f32 = 6.0;
pub const SCROLLBAR_INSET: f32 = 2.0;
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
}

#[derive(Clone, Copy, Debug)]
pub struct ScrollbarGeometry {
    pub axis: ScrollbarAxis,
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
    /// Returns thumb opacity for a given scrollbar: 0.6 if dragging, 0.5 if
    /// hovered, 0.3 otherwise. The caller builds the full `[r, g, b, a]` color.
    pub fn thumb_alpha(&self, node_id: NodeId, axis: ScrollbarAxis) -> f32 {
        if self.dragging_node == Some(node_id) && self.dragging_axis == Some(axis) {
            0.6
        } else if self.hovered_node == Some(node_id) && self.hovered_axis == Some(axis) {
            0.5
        } else {
            0.3
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

    if element.computed_style.overflow != Overflow::Scroll {
        return (None, None);
    }

    let (content_max_x, content_max_y) = content_extents(arena, node_id);
    let container_w = element.layout_rect.width;
    let container_h = element.layout_rect.height;

    let v_geom = if content_max_y > container_h + 1.0 {
        let max_scroll_y = content_max_y - container_h;
        let scroll_ratio = if max_scroll_y > 0.0 { element.scroll_y / max_scroll_y } else { 0.0 };

        let thumb_h =
            (container_h / content_max_y * container_h).max(MIN_THUMB_SIZE).min(container_h);
        let track_h = container_h;
        let thumb_y_offset = scroll_ratio * (track_h - thumb_h);

        let track_x = render_x + container_w - SCROLLBAR_WIDTH - SCROLLBAR_INSET;
        let track_y = render_y;

        Some(ScrollbarGeometry {
            axis: ScrollbarAxis::Vertical,
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

    let h_geom = if content_max_x > container_w + 1.0 {
        let max_scroll_x = content_max_x - container_w;
        let scroll_ratio = if max_scroll_x > 0.0 { element.scroll_x / max_scroll_x } else { 0.0 };

        let thumb_w =
            (container_w / content_max_x * container_w).max(MIN_THUMB_SIZE).min(container_w);
        let track_w = container_w;
        let thumb_x_offset = scroll_ratio * (track_w - thumb_w);

        let track_x = render_x;
        let track_y = render_y + container_h - SCROLLBAR_WIDTH - SCROLLBAR_INSET;

        Some(ScrollbarGeometry {
            axis: ScrollbarAxis::Horizontal,
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
            && y >= geom.track_y
            && y <= geom.track_y + geom.track_h
        {
            let part = if y >= geom.thumb_y && y <= geom.thumb_y + geom.thumb_h {
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
        if x >= geom.track_x
            && x <= geom.track_x + geom.track_w
            && y >= geom.track_y
            && y <= geom.track_y + geom.track_h
        {
            let part = if x >= geom.thumb_x && x <= geom.thumb_x + geom.thumb_w {
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

    // If this node has overflow:scroll, check its scrollbars first (they draw on top)
    if element.computed_style.overflow == Overflow::Scroll {
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
            if element.computed_style.overflow == Overflow::Scroll {
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
