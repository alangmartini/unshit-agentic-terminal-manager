use crate::id::NodeId;
use crate::style::types::{CssResize, Layer, Overflow, PointerEvents};
use crate::tree::NodeArena;
use bitflags::bitflags;
use std::fmt;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventType {
    Click,
    MouseDown,
    MouseUp,
    MouseMove,
    MouseEnter,
    MouseLeave,
    KeyDown,
    KeyUp,
    KeyboardCapture,
    Focus,
    Blur,
    Scroll,
    Clipboard,
}

/// Events related to IME (Input Method Editor) composition, used for CJK and other complex input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImeEvent {
    /// IME was enabled. The application should prepare to receive preedit and commit events.
    Enabled,
    /// A preedit (in-progress composition) string update. The optional tuple is the
    /// byte-indexed (start, end) cursor range within the preedit string.
    Preedit(String, Option<(usize, usize)>),
    /// The IME committed final text; insert it into the document.
    Commit(String),
    /// IME was disabled. Any pending preedit should be cleared.
    Disabled,
}

/// Events related to clipboard operations (copy, paste, cut).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipboardEvent {
    /// Text was copied to the clipboard.
    Copy,
    /// Text was pasted from the clipboard.
    Paste(String),
    /// Text was cut to the clipboard.
    Cut,
}

#[derive(Clone, Debug)]
pub enum Event {
    Mouse(MouseEvent),
    Keyboard(KeyboardEvent),
    Scroll(ScrollEvent),
    Clipboard(ClipboardEvent),
}

#[derive(Clone, Debug)]
pub struct MouseEvent {
    pub kind: MouseEventKind,
    pub x: f32,
    pub y: f32,
    pub button: MouseButton,
    pub modifiers: Modifiers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseEventKind {
    Down,
    Up,
    Move,
    Enter,
    Leave,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    None,
}

#[derive(Clone, Debug)]
pub struct KeyboardEvent {
    pub kind: KeyEventKind,
    pub key: Key,
    pub modifiers: Modifiers,
    pub text: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEventKind {
    Pressed,
    Released,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Char(char),
    Enter,
    Escape,
    Backspace,
    Tab,
    Space,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    PageUp,
    PageDown,
    Delete,
    F(u8),
    Unknown,
}

impl Key {
    /// Parse a key name (case-insensitive) into a `Key`.
    /// Inverse of `Display`: `Key::from_name("enter")` returns `Some(Key::Enter)`.
    pub fn from_name(s: &str) -> Option<Key> {
        let lower = s.trim().to_ascii_lowercase();
        match lower.as_str() {
            "enter" => Some(Key::Enter),
            "escape" | "esc" => Some(Key::Escape),
            "backspace" => Some(Key::Backspace),
            "tab" => Some(Key::Tab),
            "space" => Some(Key::Space),
            "up" => Some(Key::ArrowUp),
            "down" => Some(Key::ArrowDown),
            "left" => Some(Key::ArrowLeft),
            "right" => Some(Key::ArrowRight),
            "home" => Some(Key::Home),
            "end" => Some(Key::End),
            "pageup" => Some(Key::PageUp),
            "pagedown" => Some(Key::PageDown),
            "delete" | "del" => Some(Key::Delete),
            other => {
                // Try F-key pattern: "f1" through "f12"
                if let Some(n_str) = other.strip_prefix('f') {
                    if let Ok(n) = n_str.parse::<u8>() {
                        if (1..=12).contains(&n) {
                            return Some(Key::F(n));
                        }
                    }
                }
                // Single character
                let mut chars = other.chars();
                if let Some(c) = chars.next() {
                    if chars.next().is_none() {
                        return Some(Key::Char(c.to_ascii_lowercase()));
                    }
                }
                None
            }
        }
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Char(c) => write!(f, "{}", c.to_ascii_uppercase()),
            Key::Enter => write!(f, "Enter"),
            Key::Escape => write!(f, "Escape"),
            Key::Backspace => write!(f, "Backspace"),
            Key::Tab => write!(f, "Tab"),
            Key::Space => write!(f, "Space"),
            Key::ArrowUp => write!(f, "Up"),
            Key::ArrowDown => write!(f, "Down"),
            Key::ArrowLeft => write!(f, "Left"),
            Key::ArrowRight => write!(f, "Right"),
            Key::Home => write!(f, "Home"),
            Key::End => write!(f, "End"),
            Key::PageUp => write!(f, "PageUp"),
            Key::PageDown => write!(f, "PageDown"),
            Key::Delete => write!(f, "Delete"),
            Key::F(n) => write!(f, "F{}", n),
            Key::Unknown => write!(f, "Unknown"),
        }
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct Modifiers: u8 {
        const SHIFT = 0b0001;
        const CTRL  = 0b0010;
        const ALT   = 0b0100;
        const META  = 0b1000;
    }
}

impl Modifiers {
    /// Parse a modifier name (case-insensitive) into a `Modifiers` flag.
    pub fn parse_name(s: &str) -> Option<Modifiers> {
        match s.trim().to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Some(Modifiers::CTRL),
            "shift" => Some(Modifiers::SHIFT),
            "alt" => Some(Modifiers::ALT),
            "meta" | "super" | "cmd" | "command" => Some(Modifiers::META),
            _ => None,
        }
    }
}

impl fmt::Display for Modifiers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.contains(Modifiers::CTRL) {
            write!(f, "Ctrl+")?;
        }
        if self.contains(Modifiers::ALT) {
            write!(f, "Alt+")?;
        }
        if self.contains(Modifiers::SHIFT) {
            write!(f, "Shift+")?;
        }
        if self.contains(Modifiers::META) {
            write!(f, "Meta+")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct ScrollEvent {
    pub delta_x: f32,
    pub delta_y: f32,
    pub x: f32,
    pub y: f32,
}

// ---------------------------------------------------------------------------
// Drag interaction primitives
// ---------------------------------------------------------------------------

/// Default movement threshold (in pixels) before a drag begins.
/// Matches the Windows SM_CXDRAG / macOS default of 4px.
pub const DRAG_THRESHOLD: f32 = 4.0;

/// Phase of a drag interaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragPhase {
    /// The pointer moved past the threshold; drag just started.
    Start,
    /// The pointer moved while dragging.
    Update,
    /// The pointer was released; drag finished.
    End,
}

/// Data passed to drag event handlers.
/// This is a Copy struct (stack-only, no allocations per mouse move).
#[derive(Clone, Copy, Debug)]
pub struct DragEvent {
    /// Current phase of the drag.
    pub phase: DragPhase,
    /// Current cursor position (absolute, in window coordinates).
    pub x: f32,
    pub y: f32,
    /// Movement since the last DragUpdate (or since DragStart for the first update).
    pub delta_x: f32,
    pub delta_y: f32,
    /// Total movement from the drag origin.
    pub total_delta_x: f32,
    pub total_delta_y: f32,
    /// Which mouse button initiated the drag.
    pub button: MouseButton,
}

/// Represents a text selection that can span multiple elements.
#[derive(Clone, Debug)]
pub struct TextSelection {
    pub anchor_element: NodeId,
    pub anchor_offset: usize,
    pub focus_element: NodeId,
    pub focus_offset: usize,
}

impl TextSelection {
    pub fn is_collapsed(&self) -> bool {
        self.anchor_element == self.focus_element && self.anchor_offset == self.focus_offset
    }

    /// For single-element selections, returns the ordered byte range.
    /// Returns None if the selection spans multiple elements.
    pub fn single_element_range(&self) -> Option<(usize, usize)> {
        if self.anchor_element == self.focus_element {
            Some(if self.anchor_offset <= self.focus_offset {
                (self.anchor_offset, self.focus_offset)
            } else {
                (self.focus_offset, self.anchor_offset)
            })
        } else {
            None
        }
    }

    /// Backwards-compatible helper: returns ordered range when anchor and focus
    /// are in the same element. Panics if they differ (callers should prefer
    /// `single_element_range` for multi-element aware code).
    pub fn ordered_range(&self) -> (usize, usize) {
        self.single_element_range().expect("ordered_range called on a multi-element selection")
    }

    /// Given a text node and the document-order indices of anchor and focus elements,
    /// compute the selected byte range within that specific node.
    /// Returns None if the node is outside the selection range.
    pub fn element_byte_range(
        &self,
        node_id: NodeId,
        node_order: usize,
        anchor_order: usize,
        focus_order: usize,
        text_len: usize,
    ) -> Option<(usize, usize)> {
        let (start_elem, start_off, end_elem, end_off, start_order, end_order) =
            if anchor_order <= focus_order {
                (
                    self.anchor_element,
                    self.anchor_offset,
                    self.focus_element,
                    self.focus_offset,
                    anchor_order,
                    focus_order,
                )
            } else {
                (
                    self.focus_element,
                    self.focus_offset,
                    self.anchor_element,
                    self.anchor_offset,
                    focus_order,
                    anchor_order,
                )
            };

        if node_order < start_order || node_order > end_order {
            return None;
        }

        let sel_start = if node_id == start_elem { start_off } else { 0 };
        let sel_end = if node_id == end_elem { end_off } else { text_len };

        if sel_start == sel_end {
            return None;
        }

        Some((sel_start, sel_end))
    }
}

pub struct InteractionState {
    pub hovered: NodeId,
    pub focused: NodeId,
    pub active: Option<NodeId>,
    pub mousedown_target: Option<NodeId>,
    pub last_cursor_pos: (f32, f32),
    pub text_selection: Option<TextSelection>,
    pub selecting: bool,
    pub last_click_time: Option<Instant>,
    pub last_click_node: NodeId,
    pub scrollbar_drag: Option<crate::scroll::ScrollbarDrag>,
    /// Position where the mouse button was pressed (before threshold is met).
    pub drag_origin: Option<(f32, f32)>,
    /// Node whose `on_drag` handler is receiving drag events.
    pub drag_target: Option<NodeId>,
    /// Whether the drag threshold has been exceeded and DragStart was dispatched.
    pub dragging: bool,
    /// Whether the current focus was gained via keyboard (Tab) rather than mouse click.
    /// Used by `:focus-visible` pseudo-class matching.
    pub focus_via_keyboard: bool,
    /// Last position delivered to the drag handler (for computing per-move deltas).
    pub drag_last_pos: (f32, f32),
    /// Mouse button that started the drag.
    pub drag_button: MouseButton,
    /// CSS resize drag: the element being resized and its initial size.
    pub resize_drag: Option<ResizeDragInfo>,
}

/// State for a CSS `resize` drag interaction.
#[derive(Clone, Copy, Debug)]
pub struct ResizeDragInfo {
    pub node_id: NodeId,
    pub initial_width: f32,
    pub initial_height: f32,
    pub origin: (f32, f32),
    pub allow_horizontal: bool,
    pub allow_vertical: bool,
}

impl Default for InteractionState {
    fn default() -> Self {
        Self {
            hovered: NodeId::DANGLING,
            focused: NodeId::DANGLING,
            active: None,
            mousedown_target: None,
            last_cursor_pos: (0.0, 0.0),
            text_selection: None,
            selecting: false,
            last_click_time: None,
            last_click_node: NodeId::DANGLING,
            scrollbar_drag: None,
            drag_origin: None,
            drag_target: None,
            dragging: false,
            focus_via_keyboard: false,
            drag_last_pos: (0.0, 0.0),
            drag_button: MouseButton::Left,
            resize_drag: None,
        }
    }
}

pub fn hit_test(arena: &NodeArena, root: NodeId, x: f32, y: f32) -> Option<NodeId> {
    // Check layers top-to-bottom (highest layer first)
    for &layer in Layer::ALL.iter().rev() {
        // Skip tooltip layer (non-interactive)
        if layer == Layer::Tooltip {
            continue;
        }
        if let Some(hit) = hit_test_in_layer(arena, root, x, y, layer, Layer::Content) {
            return Some(hit);
        }
    }
    None
}

fn hit_test_in_layer(
    arena: &NodeArena,
    node_id: NodeId,
    x: f32,
    y: f32,
    target_layer: Layer,
    inherited_layer: Layer,
) -> Option<NodeId> {
    let element = arena.get(node_id)?;
    let rect = element.layout_rect;

    if !rect.contains(x, y) {
        return None;
    }

    // Resolve this element's effective layer
    let effective_layer = if element.computed_style.layer != Layer::Content {
        element.computed_style.layer
    } else {
        inherited_layer
    };

    // When this element scrolls, children are rendered shifted by -scroll.
    // Reverse that shift for hit testing by adding the scroll offset.
    let child_x = x + element.scroll_x;
    let child_y = y + element.scroll_y;

    // Check children in reverse order (last child = frontmost)
    let mut child = element.last_child;
    while !child.is_dangling() {
        if let Some(hit) =
            hit_test_in_layer(arena, child, child_x, child_y, target_layer, effective_layer)
        {
            return Some(hit);
        }
        child = arena.get(child).map(|e| e.prev_sibling).unwrap_or(NodeId::DANGLING);
    }

    // Only match if this element belongs to the target layer
    if effective_layer != target_layer {
        return None;
    }

    // Skip elements with pointer-events: none (let events pass through)
    if element.computed_style.pointer_events == PointerEvents::None {
        return None;
    }

    Some(node_id)
}

/// Walk up from `start` through the parent chain, returning the first node
/// that has an `on_click` handler.
pub fn find_click_handler(arena: &NodeArena, start: NodeId) -> Option<NodeId> {
    let mut current = start;
    while !current.is_dangling() {
        if let Some(element) = arena.get(current) {
            if element.on_click.is_some() {
                return Some(current);
            }
            current = element.parent;
        } else {
            break;
        }
    }
    None
}

const GRIP_ZONE: f32 = 16.0;

/// Walk the element tree looking for a resizable element whose bottom-right
/// grip zone contains the cursor. Returns `ResizeDragInfo` for the frontmost match.
pub fn find_resize_grip_at(
    arena: &NodeArena,
    root: NodeId,
    x: f32,
    y: f32,
) -> Option<ResizeDragInfo> {
    find_resize_grip_recursive(arena, root, x, y, 0.0, 0.0)
}

fn find_resize_grip_recursive(
    arena: &NodeArena,
    node_id: NodeId,
    x: f32,
    y: f32,
    scroll_offset_x: f32,
    scroll_offset_y: f32,
) -> Option<ResizeDragInfo> {
    let element = arena.get(node_id)?;
    let rect = element.layout_rect;

    let render_x = rect.x - scroll_offset_x;
    let render_y = rect.y - scroll_offset_y;

    if x < render_x || x > render_x + rect.width || y < render_y || y > render_y + rect.height {
        return None;
    }

    let child_scroll_x = scroll_offset_x + element.scroll_x;
    let child_scroll_y = scroll_offset_y + element.scroll_y;

    // Check children first (frontmost wins)
    let mut child = element.last_child;
    while !child.is_dangling() {
        if let Some(info) =
            find_resize_grip_recursive(arena, child, x, y, child_scroll_x, child_scroll_y)
        {
            return Some(info);
        }
        child = arena.get(child).map(|e| e.prev_sibling).unwrap_or(NodeId::DANGLING);
    }

    // Check this element: must have resize != None and overflow != Visible (per CSS spec)
    let style = &element.computed_style;
    if style.resize == CssResize::None || style.overflow == Overflow::Visible {
        return None;
    }

    // Check if cursor is in the bottom-right grip zone
    let grip_x = render_x + rect.width - GRIP_ZONE;
    let grip_y = render_y + rect.height - GRIP_ZONE;
    if x >= grip_x && y >= grip_y {
        let allow_horizontal = matches!(style.resize, CssResize::Both | CssResize::Horizontal);
        let allow_vertical = matches!(style.resize, CssResize::Both | CssResize::Vertical);
        return Some(ResizeDragInfo {
            node_id,
            initial_width: rect.width,
            initial_height: rect.height,
            origin: (x, y),
            allow_horizontal,
            allow_vertical,
        });
    }

    None
}

/// If a mousedown target matches the current hovered element (same element or
/// descendant), walk up the tree to find and invoke the nearest `on_click` handler.
/// Returns `true` if a handler was called.
pub fn dispatch_click(arena: &NodeArena, mousedown_target: NodeId, hovered: NodeId) -> bool {
    if !is_or_ancestor_of(arena, mousedown_target, hovered) {
        return false;
    }
    if let Some(handler_node) = find_click_handler(arena, hovered) {
        if let Some(element) = arena.get(handler_node) {
            if let Some(ref on_click) = element.on_click {
                on_click();
                return true;
            }
        }
    }
    false
}

/// Returns the DFS pre-order index of a node in the tree.
/// Returns None if target is not found under root.
pub fn document_order(arena: &NodeArena, root: NodeId, target: NodeId) -> Option<usize> {
    let mut counter = 0usize;
    if dfs_find(arena, root, target, &mut counter) {
        Some(counter)
    } else {
        None
    }
}

fn dfs_find(arena: &NodeArena, current: NodeId, target: NodeId, counter: &mut usize) -> bool {
    if current == target {
        return true;
    }
    *counter += 1;
    if let Some(element) = arena.get(current) {
        let mut child = element.first_child;
        while !child.is_dangling() {
            if dfs_find(arena, child, target, counter) {
                return true;
            }
            child = arena.get(child).map(|e| e.next_sibling).unwrap_or(NodeId::DANGLING);
        }
    }
    false
}

/// Walk up from `start` to find the nearest focusable element (or `start` itself).
pub fn find_focusable_ancestor(arena: &NodeArena, start: NodeId) -> Option<NodeId> {
    let mut current = start;
    while !current.is_dangling() {
        if let Some(element) = arena.get(current) {
            if element.is_focusable() {
                return Some(current);
            }
            current = element.parent;
        } else {
            break;
        }
    }
    None
}

/// Collect focusable node IDs in document (DFS pre-order) order.
fn collect_focusable(arena: &NodeArena, node_id: NodeId, out: &mut Vec<NodeId>) {
    if let Some(element) = arena.get(node_id) {
        if element.is_focusable() {
            out.push(node_id);
        }
        let mut child = element.first_child;
        while !child.is_dangling() {
            collect_focusable(arena, child, out);
            child = arena.get(child).map(|e| e.next_sibling).unwrap_or(NodeId::DANGLING);
        }
    }
}

/// Step through the focusable list by `offset` (+1 for next, -1 for prev),
/// wrapping around at boundaries.
fn step_focusable(
    arena: &NodeArena,
    root: NodeId,
    current: NodeId,
    offset: isize,
) -> Option<NodeId> {
    let mut focusable = Vec::new();
    collect_focusable(arena, root, &mut focusable);

    if focusable.is_empty() {
        return None;
    }

    if current.is_dangling() {
        return if offset > 0 { Some(focusable[0]) } else { Some(focusable[focusable.len() - 1]) };
    }

    let current_pos = focusable.iter().position(|id| *id == current);
    match current_pos {
        Some(pos) => {
            let len = focusable.len() as isize;
            let next = ((pos as isize + offset) % len + len) % len;
            Some(focusable[next as usize])
        }
        None => {
            if offset > 0 {
                Some(focusable[0])
            } else {
                Some(focusable[focusable.len() - 1])
            }
        }
    }
}

/// Find the next focusable element in document order after `current_focused`.
/// Wraps around to the first focusable element if at the end.
pub fn next_focusable(arena: &NodeArena, root: NodeId, current_focused: NodeId) -> Option<NodeId> {
    step_focusable(arena, root, current_focused, 1)
}

/// Find the previous focusable element in document order before `current_focused`.
/// Wraps around to the last focusable element if at the beginning.
pub fn prev_focusable(arena: &NodeArena, root: NodeId, current_focused: NodeId) -> Option<NodeId> {
    step_focusable(arena, root, current_focused, -1)
}

/// Check if `ancestor` is equal to `descendant` or is an ancestor of it.
pub fn is_or_ancestor_of(arena: &NodeArena, ancestor: NodeId, descendant: NodeId) -> bool {
    let mut current = descendant;
    while !current.is_dangling() {
        if current == ancestor {
            return true;
        }
        match arena.get(current) {
            Some(el) => current = el.parent,
            None => break,
        }
    }
    false
}

/// Given a text string and a byte offset, return the (start, end) byte offsets
/// of the word at that position. Words are separated by whitespace and punctuation.
/// If the offset lands on a separator, the returned range covers that single
/// separator character.
pub fn word_boundary_at(text: &str, byte_offset: usize) -> (usize, usize) {
    if text.is_empty() {
        return (0, 0);
    }
    let offset = byte_offset.min(text.len());

    // If offset is at the end, step back to the last character
    let idx = if offset == text.len() && offset > 0 {
        text.floor_char_boundary(offset - 1)
    } else {
        text.floor_char_boundary(offset)
    };

    let ch = match text[idx..].chars().next() {
        Some(c) => c,
        None => return (offset, offset),
    };

    let is_word_char = |c: char| c.is_alphanumeric() || c == '_';

    if is_word_char(ch) {
        // Scan backward to word start
        let start = text[..idx]
            .char_indices()
            .rev()
            .take_while(|&(_, c)| is_word_char(c))
            .last()
            .map(|(i, _)| i)
            .unwrap_or(idx);

        // Scan forward to word end
        let end = text[idx..]
            .char_indices()
            .take_while(|&(_, c)| is_word_char(c))
            .last()
            .map(|(i, c)| idx + i + c.len_utf8())
            .unwrap_or(idx);

        (start, end)
    } else {
        // Non-word character: select just that character
        (idx, idx + ch.len_utf8())
    }
}

/// Walk up from `start` through the parent chain, returning the first node
/// that has an `on_drag` handler.
pub fn find_drag_handler(arena: &NodeArena, start: NodeId) -> Option<NodeId> {
    let mut current = start;
    while !current.is_dangling() {
        if let Some(element) = arena.get(current) {
            if element.on_drag.is_some() {
                return Some(current);
            }
            current = element.parent;
        } else {
            break;
        }
    }
    None
}

/// Walk up from `start` through the parent chain, returning the first node
/// that has an `on_context_menu` handler.
pub fn find_context_menu_handler(arena: &NodeArena, start: NodeId) -> Option<NodeId> {
    let mut current = start;
    while !current.is_dangling() {
        if let Some(element) = arena.get(current) {
            if element.on_context_menu.is_some() {
                return Some(current);
            }
            current = element.parent;
        } else {
            break;
        }
    }
    None
}

/// Walk up the tree from the hovered element to find and invoke the nearest
/// `on_context_menu` handler. Returns `true` if a handler was called.
pub fn dispatch_context_menu(arena: &NodeArena, hovered: NodeId) -> bool {
    if let Some(handler_node) = find_context_menu_handler(arena, hovered) {
        if let Some(element) = arena.get(handler_node) {
            if let Some(ref on_context_menu) = element.on_context_menu {
                on_context_menu();
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ime_event_enabled_variant() {
        let event = ImeEvent::Enabled;
        assert_eq!(event, ImeEvent::Enabled);
    }

    #[test]
    fn test_ime_event_preedit_variant() {
        let event = ImeEvent::Preedit("あ".to_string(), Some((0, 3)));
        assert_eq!(event, ImeEvent::Preedit("あ".to_string(), Some((0, 3))));
    }

    #[test]
    fn test_ime_event_preedit_no_cursor() {
        let event = ImeEvent::Preedit("abc".to_string(), None);
        assert_eq!(event, ImeEvent::Preedit("abc".to_string(), None));
    }

    #[test]
    fn test_ime_event_commit_variant() {
        let event = ImeEvent::Commit("世界".to_string());
        assert_eq!(event, ImeEvent::Commit("世界".to_string()));
    }

    #[test]
    fn test_ime_event_disabled_variant() {
        let event = ImeEvent::Disabled;
        assert_eq!(event, ImeEvent::Disabled);
    }

    #[test]
    fn test_ime_event_clone() {
        let original = ImeEvent::Preedit("日本語".to_string(), Some((0, 9)));
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    // -----------------------------------------------------------------------
    // find_resize_grip_at tests
    // -----------------------------------------------------------------------

    use crate::element::{Element, LayoutRect, Tag};
    use crate::style::types::{CssResize as Resize, Overflow};
    use crate::tree::NodeArena;

    /// Build a single resizable element at a known position.
    fn make_resizable_element(
        arena: &mut NodeArena,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        resize: Resize,
        overflow: Overflow,
    ) -> NodeId {
        let mut elem = Element::new(Tag::Div);
        elem.layout_rect = LayoutRect { x, y, width: w, height: h };
        elem.computed_style.resize = resize;
        elem.computed_style.overflow = overflow;
        arena.alloc(elem)
    }

    #[test]
    fn resize_grip_requires_overflow_not_visible() {
        let mut arena = NodeArena::new();
        let root = make_resizable_element(
            &mut arena,
            0.0,
            0.0,
            200.0,
            200.0,
            Resize::Both,
            Overflow::Visible,
        );
        // Cursor in grip zone but overflow is Visible: no hit
        assert!(find_resize_grip_at(&arena, root, 195.0, 195.0).is_none());
    }

    #[test]
    fn resize_grip_requires_resize_not_none() {
        let mut arena = NodeArena::new();
        let root = make_resizable_element(
            &mut arena,
            0.0,
            0.0,
            200.0,
            200.0,
            Resize::None,
            Overflow::Hidden,
        );
        // Overflow is hidden but resize is None: no hit
        assert!(find_resize_grip_at(&arena, root, 195.0, 195.0).is_none());
    }

    #[test]
    fn resize_grip_hit_bottom_right() {
        let mut arena = NodeArena::new();
        let root = make_resizable_element(
            &mut arena,
            0.0,
            0.0,
            200.0,
            200.0,
            Resize::Both,
            Overflow::Hidden,
        );
        // Cursor in bottom-right 16x16 grip zone
        let info = find_resize_grip_at(&arena, root, 190.0, 190.0);
        assert!(info.is_some());
        let info = info.unwrap();
        assert_eq!(info.node_id, root);
        assert!(info.allow_horizontal);
        assert!(info.allow_vertical);
        assert_eq!(info.initial_width, 200.0);
        assert_eq!(info.initial_height, 200.0);
    }

    #[test]
    fn resize_grip_miss_outside_zone() {
        let mut arena = NodeArena::new();
        let root = make_resizable_element(
            &mut arena,
            0.0,
            0.0,
            200.0,
            200.0,
            Resize::Both,
            Overflow::Hidden,
        );
        // Cursor in the center of the element: not in grip zone
        assert!(find_resize_grip_at(&arena, root, 100.0, 100.0).is_none());
    }

    #[test]
    fn resize_grip_horizontal_only() {
        let mut arena = NodeArena::new();
        let root = make_resizable_element(
            &mut arena,
            0.0,
            0.0,
            200.0,
            200.0,
            Resize::Horizontal,
            Overflow::Scroll,
        );
        let info = find_resize_grip_at(&arena, root, 195.0, 195.0).unwrap();
        assert!(info.allow_horizontal);
        assert!(!info.allow_vertical);
    }

    #[test]
    fn resize_grip_vertical_only() {
        let mut arena = NodeArena::new();
        let root = make_resizable_element(
            &mut arena,
            0.0,
            0.0,
            200.0,
            200.0,
            Resize::Vertical,
            Overflow::Scroll,
        );
        let info = find_resize_grip_at(&arena, root, 195.0, 195.0).unwrap();
        assert!(!info.allow_horizontal);
        assert!(info.allow_vertical);
    }

    #[test]
    fn resize_grip_child_takes_priority() {
        let mut arena = NodeArena::new();
        // Parent is resizable
        let parent = make_resizable_element(
            &mut arena,
            0.0,
            0.0,
            300.0,
            300.0,
            Resize::Both,
            Overflow::Hidden,
        );
        // Child overlaps parent's grip zone and is also resizable
        let child = make_resizable_element(
            &mut arena,
            100.0,
            100.0,
            200.0,
            200.0,
            Resize::Both,
            Overflow::Hidden,
        );
        arena.append_child(parent, child);

        // Click at (295, 295): in child's grip zone (child ends at 300,300, grip starts at 284,284)
        let info = find_resize_grip_at(&arena, parent, 295.0, 295.0).unwrap();
        assert_eq!(info.node_id, child);
    }
}
