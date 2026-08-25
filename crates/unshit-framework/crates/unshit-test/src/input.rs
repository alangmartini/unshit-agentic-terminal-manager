use std::time::Instant;
use unshit_app::app::{apply_scroll_grid_patch, ScrollGridPatch};
use unshit_core::element::{ElementContent, InputType, Tag};
use unshit_core::event::{
    dispatch_click, dispatch_context_menu, find_drag_handler, find_focusable_ancestor, hit_test,
    next_focusable, prev_focusable, word_boundary_at, DragEvent, DragPhase, Event, EventType,
    Modifiers, MouseButton, ScrollEvent, TextSelection, DRAG_THRESHOLD,
};
use unshit_core::id::NodeId;
use unshit_core::layout;
use unshit_core::scroll::{self, ScrollbarAxis, ScrollbarPart};

use crate::TestHarness;

/// Cursor position relative to a node's content box (padding box origin),
/// mirroring the production app's `local_pointer_coords` so simulated drags
/// carry the same `local_x`/`local_y` the real input pipeline would. Falls
/// back to the window coordinates when the node is missing.
fn drag_local_coords(
    arena: &unshit_core::tree::NodeArena,
    node: NodeId,
    x: f32,
    y: f32,
) -> (f32, f32) {
    if let Some(el) = arena.get(node) {
        let r = el.layout_rect;
        let p = &el.computed_style.padding;
        (x - r.x - p.left, y - r.y - p.top)
    } else {
        (x, y)
    }
}

impl TestHarness {
    /// Simulate mouse movement to (x, y). Updates hover state and marks
    /// restyle if the hovered element changed.
    pub fn mouse_move(&mut self, x: f32, y: f32) {
        // Handle active scrollbar drag
        if let Some(ref drag) = self.interaction.scrollbar_drag {
            let drag = *drag;
            let cursor_pos = match drag.axis {
                ScrollbarAxis::Vertical => y,
                ScrollbarAxis::Horizontal => x,
            };
            let new_scroll = scroll::scroll_from_drag(&drag, cursor_pos);
            scroll::set_axis_scroll_position(&mut self.arena, drag.node_id, drag.axis, new_scroll);
            self.interaction.last_cursor_pos = (x, y);
            return;
        }

        // Drag threshold check: if origin is set but drag has not started yet,
        // check whether the pointer has moved far enough to begin a drag.
        if let Some(origin) = self.interaction.drag_origin {
            if !self.interaction.dragging {
                let dx = x - origin.0;
                let dy = y - origin.1;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist >= DRAG_THRESHOLD {
                    // Try to find an on_drag handler walking up from mousedown_target
                    let target = self.interaction.mousedown_target.unwrap_or(NodeId::DANGLING);
                    if let Some(handler_node) = find_drag_handler(&self.arena, target) {
                        self.interaction.drag_target = Some(handler_node);
                        self.interaction.dragging = true;
                        self.interaction.drag_last_pos = origin;

                        // Dispatch DragStart
                        let (local_x, local_y) = drag_local_coords(&self.arena, handler_node, x, y);
                        let event = DragEvent {
                            phase: DragPhase::Start,
                            x,
                            y,
                            local_x,
                            local_y,
                            delta_x: dx,
                            delta_y: dy,
                            total_delta_x: dx,
                            total_delta_y: dy,
                            button: self.interaction.drag_button,
                        };
                        if let Some(element) = self.arena.get(handler_node) {
                            if let Some(ref on_drag) = element.on_drag {
                                on_drag(&event);
                            }
                        }
                        self.interaction.drag_last_pos = (x, y);
                    } else {
                        // No handler found; clear drag_origin so we stop checking
                        self.interaction.drag_origin = None;
                    }
                }
            }
        }

        // Active drag: dispatch DragUpdate (pointer captured to drag target)
        if self.interaction.dragging {
            if let Some(handler_node) = self.interaction.drag_target {
                let origin = self.interaction.drag_origin.unwrap_or((x, y));
                let last = self.interaction.drag_last_pos;
                let (local_x, local_y) = drag_local_coords(&self.arena, handler_node, x, y);
                let event = DragEvent {
                    phase: DragPhase::Update,
                    x,
                    y,
                    local_x,
                    local_y,
                    delta_x: x - last.0,
                    delta_y: y - last.1,
                    total_delta_x: x - origin.0,
                    total_delta_y: y - origin.1,
                    button: self.interaction.drag_button,
                };
                if let Some(element) = self.arena.get(handler_node) {
                    if let Some(ref on_drag) = element.on_drag {
                        on_drag(&event);
                    }
                }
                self.interaction.drag_last_pos = (x, y);
            }
            self.interaction.last_cursor_pos = (x, y);
            // During drag, skip normal hover updates (pointer is captured)
            return;
        }

        self.refresh_hover(x, y);
    }

    /// Recompute scrollbar hover and the hovered node for the pointer at
    /// (x, y), mirroring the production app's `handle_normal_hover`. Split
    /// out of [`TestHarness::mouse_move`] so the wheel path can re-run it
    /// after a container scroll moves content under a stationary cursor.
    fn refresh_hover(&mut self, x: f32, y: f32) {
        // Check scrollbar hover
        let sb_hit = scroll::find_scrollbar_at(&self.arena, self.root, x, y);
        self.scrollbar_visual.set_hover(sb_hit.as_ref());

        let new_hover = hit_test(&self.arena, self.root, x, y).unwrap_or(NodeId::DANGLING);

        if new_hover != self.interaction.hovered {
            self.interaction.hovered = new_hover;
            self.needs_restyle = true;
        }

        self.interaction.last_cursor_pos = (x, y);
    }

    /// Simulate a mouse button press at (x, y).
    pub fn mouse_down(&mut self, x: f32, y: f32) {
        self.mouse_move(x, y);

        // Check scrollbar hit first
        if let Some(hit) = scroll::find_scrollbar_at(&self.arena, self.root, x, y) {
            match hit.part {
                ScrollbarPart::Thumb => {
                    let grab_offset = match hit.axis {
                        ScrollbarAxis::Vertical => y - hit.geometry.thumb_y,
                        ScrollbarAxis::Horizontal => x - hit.geometry.thumb_x,
                    };
                    self.interaction.scrollbar_drag = Some(scroll::ScrollbarDrag {
                        node_id: hit.node_id,
                        axis: hit.axis,
                        grab_offset,
                        geometry: hit.geometry,
                    });
                    self.scrollbar_visual.dragging_node = Some(hit.node_id);
                    self.scrollbar_visual.dragging_axis = Some(hit.axis);
                }
                ScrollbarPart::TrackBefore | ScrollbarPart::TrackAfter => {
                    let cursor_pos = match hit.axis {
                        ScrollbarAxis::Vertical => y,
                        ScrollbarAxis::Horizontal => x,
                    };
                    let new_scroll = scroll::scroll_from_track_click(&hit.geometry, cursor_pos);
                    scroll::set_axis_scroll_position(
                        &mut self.arena,
                        hit.node_id,
                        hit.axis,
                        new_scroll,
                    );
                }
            }
            return;
        }

        // Begin potential drag: record origin for threshold check
        self.interaction.drag_origin = Some((x, y));
        self.interaction.drag_button = MouseButton::Left;
        self.interaction.dragging = false;
        self.interaction.drag_target = None;

        let now = Instant::now();
        let hovered = self.interaction.hovered;
        let is_double_click = if let Some(prev_time) = self.interaction.last_click_time {
            now.duration_since(prev_time).as_millis() < 500
                && self.interaction.last_click_node == hovered
        } else {
            false
        };

        self.interaction.last_click_time = Some(now);
        self.interaction.last_click_node = hovered;

        if !hovered.is_dangling() {
            self.interaction.active = Some(hovered);
            self.interaction.mousedown_target = Some(hovered);
            self.needs_restyle = true;
        }

        let new_focused = find_focusable_ancestor(&self.arena, self.interaction.hovered)
            .unwrap_or(NodeId::DANGLING);
        if new_focused != self.interaction.focused {
            self.interaction.focused = new_focused;
            self.interaction.focus_via_keyboard = false;
            self.needs_restyle = true;
        }

        // Text selection: start on mousedown over text
        if let Some((text_node, byte_offset)) =
            layout::text_hit_at(&self.arena, hovered, x, y, &mut self.font_system)
        {
            if is_double_click {
                // Double-click: select the word at the click position
                if let Some(elem) = self.arena.get(text_node) {
                    if let ElementContent::Text(ref text) = elem.content {
                        let (start, end) = word_boundary_at(text, byte_offset);
                        self.interaction.text_selection = Some(TextSelection {
                            anchor_element: text_node,
                            anchor_offset: start,
                            focus_element: text_node,
                            focus_offset: end,
                        });
                        // Reset last_click_time so a third click is not another double-click
                        self.interaction.last_click_time = None;
                    }
                }
                self.interaction.selecting = false;
            } else {
                self.interaction.text_selection = Some(TextSelection {
                    anchor_element: text_node,
                    anchor_offset: byte_offset,
                    focus_element: text_node,
                    focus_offset: byte_offset,
                });
                self.interaction.selecting = true;
            }
        } else {
            self.interaction.text_selection = None;
            self.interaction.selecting = false;
        }
    }

    /// Simulate a mouse button release at (x, y).
    pub fn mouse_up(&mut self, x: f32, y: f32) {
        if self.interaction.scrollbar_drag.is_some() {
            self.interaction.scrollbar_drag = None;
            self.scrollbar_visual.clear_drag();
            self.interaction.last_cursor_pos = (x, y);
            return;
        }

        // If a drag was active, dispatch DragEnd and suppress click
        if self.interaction.dragging {
            if let Some(handler_node) = self.interaction.drag_target {
                let origin = self.interaction.drag_origin.unwrap_or((x, y));
                let last = self.interaction.drag_last_pos;
                let (local_x, local_y) = drag_local_coords(&self.arena, handler_node, x, y);
                let event = DragEvent {
                    phase: DragPhase::End,
                    x,
                    y,
                    local_x,
                    local_y,
                    delta_x: x - last.0,
                    delta_y: y - last.1,
                    total_delta_x: x - origin.0,
                    total_delta_y: y - origin.1,
                    button: self.interaction.drag_button,
                };
                if let Some(element) = self.arena.get(handler_node) {
                    if let Some(ref on_drag) = element.on_drag {
                        on_drag(&event);
                    }
                }
            }
            // Clear drag state
            self.interaction.drag_origin = None;
            self.interaction.drag_target = None;
            self.interaction.dragging = false;
            // Consume the mousedown target so click does NOT fire
            self.interaction.mousedown_target = None;
            self.interaction.selecting = false;
            if self.interaction.active.is_some() {
                self.interaction.active = None;
                self.needs_restyle = true;
            }
            self.interaction.last_cursor_pos = (x, y);
            return;
        }

        // Clear drag origin (no drag occurred)
        self.interaction.drag_origin = None;

        self.mouse_move(x, y);

        if let Some(mousedown_target) = self.interaction.mousedown_target.take() {
            // Handle checkbox/radio before generic click dispatch.
            let input_handled = self.handle_input_element_click(mousedown_target);
            if !input_handled {
                dispatch_click(&self.arena, mousedown_target, self.interaction.hovered);
            }
        }

        self.interaction.selecting = false;

        if self.interaction.active.is_some() {
            self.interaction.active = None;
            self.needs_restyle = true;
        }
    }

    /// Simulate a full click (mouse_down, step, mouse_up, step).
    pub fn click(&mut self, x: f32, y: f32) {
        self.mouse_down(x, y);
        self.step();
        self.mouse_up(x, y);
        self.step();
    }

    /// Simulate a double-click (two rapid clicks at the same position).
    pub fn double_click(&mut self, x: f32, y: f32) {
        self.mouse_down(x, y);
        self.step();
        self.mouse_up(x, y);
        self.step();
        // Second click immediately triggers double-click detection
        self.mouse_down(x, y);
        self.step();
        self.mouse_up(x, y);
        self.step();
    }

    /// Simulate a right-click at (x, y). Dispatches the context menu event
    /// by walking up the parent chain for an `on_context_menu` handler.
    pub fn right_click(&mut self, x: f32, y: f32) {
        self.mouse_move(x, y);
        dispatch_context_menu(&self.arena, self.interaction.hovered, x, y);
        self.step();
    }

    /// Simulate a mouse wheel event at (x, y) with the given delta.
    ///
    /// Mirrors both halves of the production app's `WindowEvent::MouseWheel`
    /// path, in the same order:
    ///
    /// 1. **Container scrolling.** Walks up from the hovered element to find
    ///    a scroll container (`overflow: scroll` / `auto`) and applies the
    ///    delta, clamped to content bounds. A scroll that actually moved
    ///    refreshes hover, because the node under a stationary cursor may
    ///    have changed.
    /// 2. **Element dispatch.** Builds a `ScrollEvent` and walks from the
    ///    hovered element up to the root, firing the *first* element
    ///    carrying an `EventType::Scroll` handler and then stopping (bubble
    ///    semantics; the walk also stops at a dangling parent). A handler
    ///    returning a boxed `ScrollGridPatch` whose `grid` is set has that
    ///    grid written onto the handling node through the app's own
    ///    `apply_scroll_grid_patch`, so harness and app cannot drift.
    ///
    /// Dispatch is hover-driven, never focus-driven: the handler under the
    /// pointer fires even when a different element holds focus.
    ///
    /// # What this deliberately does not replicate
    ///
    /// The harness is synchronous and has no clock, window, or app config,
    /// so it reproduces only the deterministic subset of the real path:
    ///
    /// - **No animation.** The event is always built with `animate: false`,
    ///   `smooth_duration_ms: 0.0` and `smooth_initial_slope: 0.0`, so
    ///   handlers take their instant path. Container smooth scrolling is
    ///   skipped for the same reason: `scroll_by` lands the delta at once.
    /// - **No grid-animation hooks.** A `ScrollGridPatch::animation` hook is
    ///   dropped rather than registered; there is no animation tick to
    ///   drive it and no clock to sample it with.
    /// - **No rebuild fallback.** The app sets `needs_rebuild` when a
    ///   handler returns something other than a `ScrollGridPatch`, or when
    ///   the patch cannot be applied in place (node gone, non-grid content,
    ///   changed dimensions). The harness never rebuilds - it does not
    ///   retain the tree builder - so those cases are no-ops here.
    /// - **No modifiers.** `modifiers` is always empty; the harness tracks
    ///   no live modifier state, so modifier-gated handler branches (for
    ///   example Shift+wheel) are not reachable through this entry point.
    /// - **Raw deltas.** `delta_x` / `delta_y` are passed through as given:
    ///   no `LineDelta`-to-pixel conversion, no scale or zoom factor, no
    ///   scroll tuning, and no Ctrl+wheel zoom branch.
    /// - **No container axis remap.** The app runs container deltas through
    ///   `wheel_delta_for_container`, so a vertical wheel over a
    ///   horizontally-overflowing container moves it sideways. The harness
    ///   applies the raw deltas to the container. (Pre-existing gap; the
    ///   element event carries raw deltas in both.)
    /// - **No telemetry and no redraw requests.**
    pub fn mouse_wheel(&mut self, x: f32, y: f32, delta_x: f32, delta_y: f32) {
        self.mouse_move(x, y);

        let scroll_target = scroll::find_scroll_container(&self.arena, self.interaction.hovered);

        if let Some(target_id) = scroll_target {
            if scroll::scroll_by(&mut self.arena, &self.taffy, target_id, delta_x, delta_y) {
                self.refresh_hover(x, y);
            }
        }

        // Dispatch Scroll to element handlers, matching the app's walk.
        let pos = self.interaction.last_cursor_pos;
        let scroll_evt = Event::Scroll(ScrollEvent {
            delta_x,
            delta_y,
            x: pos.0,
            y: pos.1,
            animate: false,
            smooth_duration_ms: 0.0,
            smooth_initial_slope: 0.0,
            modifiers: Modifiers::empty(),
        });

        let mut node = self.interaction.hovered;
        while let Some(element) = self.arena.get(node) {
            let handler = element
                .handlers
                .iter()
                .find(|(et, _)| *et == EventType::Scroll)
                .map(|(_, handler)| handler.clone());
            if let Some(handler) = handler {
                if let Some(patch) =
                    handler(&scroll_evt).and_then(|value| value.downcast::<ScrollGridPatch>().ok())
                {
                    if let Some(grid) = patch.grid {
                        // A `false` return is where the app would fall back
                        // to a full rebuild; the harness has no rebuild.
                        apply_scroll_grid_patch(&mut self.arena, node, grid);
                    }
                    // `patch.animation` is intentionally dropped: see the
                    // fidelity notes above.
                }
                break;
            }
            let parent = element.parent;
            if parent.is_dangling() {
                break;
            }
            node = parent;
        }
    }

    /// Simulate a text selection drag from (start_x, start_y) to (end_x, end_y).
    pub fn select_text(&mut self, start_x: f32, start_y: f32, end_x: f32, end_y: f32) {
        self.mouse_down(start_x, start_y);
        self.step();

        // Move to end and extend selection
        self.mouse_move(end_x, end_y);
        if self.interaction.selecting {
            if let Some((text_node, byte_offset)) = layout::nearest_text_hit_at(
                &self.arena,
                self.root,
                end_x,
                end_y,
                &mut self.font_system,
            ) {
                if let Some(ref mut sel) = self.interaction.text_selection {
                    sel.focus_element = text_node;
                    sel.focus_offset = byte_offset;
                }
            }
        }
        self.step();

        // Release
        self.interaction.selecting = false;
        if self.interaction.active.is_some() {
            self.interaction.active = None;
            self.needs_restyle = true;
        }
        self.step();
    }

    /// Directly set focus to a specific node. Marks restyle if focus changed.
    pub fn focus(&mut self, node_id: NodeId) {
        if node_id != self.interaction.focused {
            let old_focused = self.interaction.focused;
            self.interaction.focused = node_id;
            self.clear_input_selection_on_blur(old_focused);
            self.needs_restyle = true;
        }
    }

    /// Simulate pressing Tab: move focus to the next focusable element.
    pub fn tab(&mut self) {
        if let Some(id) = next_focusable(&self.arena, self.root, self.interaction.focused) {
            if id != self.interaction.focused {
                let old_focused = self.interaction.focused;
                self.interaction.focused = id;
                self.interaction.focus_via_keyboard = true;
                self.clear_input_selection_on_blur(old_focused);
                self.needs_restyle = true;
            }
        }
    }

    /// Simulate pressing Shift+Tab: move focus to the previous focusable element.
    pub fn shift_tab(&mut self) {
        if let Some(id) = prev_focusable(&self.arena, self.root, self.interaction.focused) {
            if id != self.interaction.focused {
                let old_focused = self.interaction.focused;
                self.interaction.focused = id;
                self.interaction.focus_via_keyboard = true;
                self.clear_input_selection_on_blur(old_focused);
                self.needs_restyle = true;
            }
        }
    }

    /// Mirror the production app: an input losing focus drops any active
    /// text selection so a later refocus starts clean.
    fn clear_input_selection_on_blur(&mut self, node: NodeId) {
        if let Some(element) = self.arena.get_mut(node) {
            if element.tag == Tag::Input {
                element.input_state.selection_anchor = None;
            }
        }
    }

    /// Assert that the hovered element remains stable for `frames` consecutive
    /// steps. Panics with a descriptive message if hover oscillates.
    pub fn assert_hover_stable(&mut self, frames: usize) {
        let initial = self.interaction.hovered;
        for i in 0..frames {
            self.step();
            let current = self.interaction.hovered;
            if current != initial {
                panic!(
                    "Hover oscillated on frame {}: expected {:?} but got {:?}",
                    i + 1,
                    initial,
                    current,
                );
            }
        }
    }

    /// Type a single character into the focused input element.
    pub fn type_char(&mut self, ch: char) {
        let focused = self.interaction.focused;
        let Some(element) = self.arena.get_mut(focused) else {
            return;
        };
        if element.tag != Tag::Input {
            return;
        }

        let old_value = element.input_state.value.clone();
        unshit_core::input::insert_text_filtered(&mut element.input_state, &ch.to_string());
        element.cursor_state.reset_blink(Instant::now());
        let new_value = element.input_state.value.clone();
        let on_change = element.on_change.clone();

        if new_value != old_value {
            if let Some(f) = on_change {
                f(&new_value);
            }
        }
        self.needs_relayout = true;
    }

    /// Type a string into the focused input element.
    pub fn type_text(&mut self, text: &str) {
        for ch in text.chars() {
            self.type_char(ch);
        }
    }

    /// Press a special key on the focused input element.
    pub fn press_key(&mut self, key: unshit_core::event::Key) {
        self.press_key_with_mods(key, false, false);
    }

    /// Press a key with modifier context, mirroring the production app's
    /// `handle_text_input`: `shift` extends the selection on movement keys,
    /// `ctrl` switches movement/deletion to word granularity.
    pub fn press_key_with_mods(&mut self, key: unshit_core::event::Key, shift: bool, ctrl: bool) {
        let focused = self.interaction.focused;
        let Some(element) = self.arena.get_mut(focused) else {
            return;
        };
        if element.tag != Tag::Input {
            return;
        }

        if key == unshit_core::event::Key::Enter {
            let input_type = element.input_state.input_type;
            // Clamp number inputs on Enter before firing on_submit.
            if input_type == InputType::Number {
                unshit_core::input::clamp_number_input(&mut element.input_state);
            }
            let on_submit = element.on_submit.clone();
            let value = element.input_state.value.clone();
            if let Some(f) = on_submit {
                f(&value);
            }
            self.needs_relayout = true;
            return;
        }

        let old_value = element.input_state.value.clone();
        unshit_core::input::apply_key_with_mods(&mut element.input_state, &key, shift, ctrl);
        element.cursor_state.reset_blink(Instant::now());
        let changed = element.input_state.value != old_value;
        let new_value = element.input_state.value.clone();
        let on_change = element.on_change.clone();

        if changed {
            if let Some(f) = on_change {
                f(&new_value);
            }
        }
        self.needs_relayout = true;
    }

    /// Select the entire value of the focused input (Ctrl+A).
    pub fn select_all(&mut self) {
        let focused = self.interaction.focused;
        let Some(element) = self.arena.get_mut(focused) else {
            return;
        };
        if element.tag != Tag::Input {
            return;
        }
        unshit_core::input::select_all(&mut element.input_state);
        element.cursor_state.reset_blink(Instant::now());
        self.needs_relayout = true;
    }

    /// Handle a click on a checkbox or radio input. Returns true if consumed.
    pub(crate) fn handle_input_element_click(&mut self, target: NodeId) -> bool {
        let Some(element) = self.arena.get(target) else { return false };
        if element.tag != Tag::Input {
            return false;
        }
        match element.input_state.input_type {
            InputType::Checkbox => {
                let (new_checked, on_change) = {
                    let elem = self.arena.get_mut(target).unwrap();
                    elem.input_state.checked = !elem.input_state.checked;
                    (elem.input_state.checked, elem.on_change.clone())
                };
                if let Some(f) = on_change {
                    f(if new_checked { "true" } else { "false" });
                }
                true
            }
            InputType::Radio => {
                let already_checked =
                    self.arena.get(target).map(|e| e.input_state.checked).unwrap_or(false);
                if !already_checked {
                    let radio_name = self.arena.get(target).and_then(|e| e.name.clone());
                    // Uncheck all other radios in the same group.
                    let siblings: Vec<NodeId> = self
                        .arena
                        .iter()
                        .filter(|(id, elem)| {
                            *id != target
                                && elem.tag == Tag::Input
                                && elem.input_state.input_type == InputType::Radio
                                && radio_name.is_some()
                                && elem.name.as_deref() == radio_name.as_deref()
                        })
                        .map(|(id, _)| id)
                        .collect();
                    for sid in siblings {
                        if let Some(e) = self.arena.get_mut(sid) {
                            e.input_state.checked = false;
                        }
                    }
                    let on_change = if let Some(e) = self.arena.get_mut(target) {
                        e.input_state.checked = true;
                        e.on_change.clone()
                    } else {
                        None
                    };
                    if let Some(f) = on_change {
                        f("true");
                    }
                }
                true
            }
            _ => false,
        }
    }

    /// Get the current value of the focused input element.
    pub fn input_value(&self) -> Option<String> {
        let element = self.arena.get(self.interaction.focused)?;
        if element.tag != Tag::Input {
            return None;
        }
        Some(element.input_state.value.clone())
    }

    /// Get the cursor position of the focused input element.
    pub fn input_cursor_pos(&self) -> Option<usize> {
        let element = self.arena.get(self.interaction.focused)?;
        if element.tag != Tag::Input {
            return None;
        }
        Some(element.input_state.cursor_pos)
    }

    /// Ordered byte range of the focused input's active selection, or
    /// `None` when the focused element is not an input or nothing is
    /// selected.
    pub fn input_selection(&self) -> Option<(usize, usize)> {
        let element = self.arena.get(self.interaction.focused)?;
        if element.tag != Tag::Input {
            return None;
        }
        unshit_core::input::selection_range(&element.input_state)
    }
}
