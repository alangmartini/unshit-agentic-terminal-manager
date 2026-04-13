//! High-level action API for the test harness.
//!
//! These methods accept CSS selectors (`.class`, `#id`, `tagname`) instead
//! of raw coordinates, making tests more readable and less brittle.

use std::time::Instant;

use unshit_core::element::Tag;
use unshit_core::event::Key;

use crate::trace::TraceAction;
use crate::TestHarness;

impl TestHarness {
    /// Resolve a selector to an `ElementSnapshot`.
    /// Panics with a clear message if no element matches.
    fn resolve(&self, selector: &str) -> crate::ElementSnapshot {
        self.query(selector)
            .unwrap_or_else(|| panic!("action failed: no element matches selector '{}'", selector))
    }

    /// Resolve a selector to the matching element's center coordinates.
    fn resolve_center(&self, selector: &str) -> (f32, f32) {
        let r = self.resolve(selector).layout_rect;
        (r.x + r.width / 2.0, r.y + r.height / 2.0)
    }

    /// Click on the first element matching `selector`.
    ///
    /// Resolves the element center, performs mouse_down + step + mouse_up + step.
    pub fn click_on(&mut self, selector: &str) {
        let (cx, cy) = self.resolve_center(selector);
        self.trace.record(TraceAction::Click { selector: selector.to_owned(), x: cx, y: cy });
        self.click(cx, cy);
    }

    /// Double-click on the first element matching `selector`.
    pub fn double_click_on(&mut self, selector: &str) {
        let (cx, cy) = self.resolve_center(selector);
        self.trace.record(TraceAction::DoubleClick { selector: selector.to_owned(), x: cx, y: cy });
        self.double_click(cx, cy);
    }

    /// Right-click (context menu) on the first element matching `selector`.
    pub fn right_click_on(&mut self, selector: &str) {
        let (cx, cy) = self.resolve_center(selector);
        self.trace.record(TraceAction::RightClick { selector: selector.to_owned(), x: cx, y: cy });
        self.right_click(cx, cy);
    }

    /// Move the mouse to the center of the first element matching `selector`
    /// and advance one frame so :hover styles resolve.
    pub fn hover_on(&mut self, selector: &str) {
        let (cx, cy) = self.resolve_center(selector);
        self.trace.record(TraceAction::Hover { selector: selector.to_owned(), x: cx, y: cy });
        self.mouse_move(cx, cy);
        self.step();
    }

    /// Scroll on the first element matching `selector` with the given deltas.
    pub fn scroll_on(&mut self, selector: &str, delta_x: f32, delta_y: f32) {
        self.trace.record(TraceAction::Scroll {
            selector: selector.to_owned(),
            dx: delta_x,
            dy: delta_y,
        });
        let (cx, cy) = self.resolve_center(selector);
        self.mouse_wheel(cx, cy, delta_x, delta_y);
        self.step();
    }

    /// Focus the input matching `selector`, clear its value, and type `text`.
    ///
    /// Simulates a user clicking the field, clearing it, then typing
    /// the new value.
    pub fn fill(&mut self, selector: &str, text: &str) {
        self.trace
            .record(TraceAction::Fill { selector: selector.to_owned(), text: text.to_owned() });
        let snap = self.resolve(selector);
        let node_id = snap.node_id;
        let r = snap.layout_rect;
        self.click(r.x + r.width / 2.0, r.y + r.height / 2.0);
        self.clear_input(node_id);
        self.type_text(text);
        self.step();
    }

    /// Clear the input matching `selector`.
    pub fn clear(&mut self, selector: &str) {
        self.trace.record(TraceAction::Clear { selector: selector.to_owned() });
        let snap = self.resolve(selector);
        let node_id = snap.node_id;
        let r = snap.layout_rect;
        self.click(r.x + r.width / 2.0, r.y + r.height / 2.0);
        self.clear_input(node_id);
        self.step();
    }

    /// Clear the value of an input element and fire `on_change` if non-empty.
    pub(crate) fn clear_input(&mut self, node_id: unshit_core::id::NodeId) {
        if let Some(element) = self.arena.get_mut(node_id) {
            if element.tag == Tag::Input {
                let old_value = element.input_state.value.clone();
                element.input_state.value.clear();
                element.input_state.cursor_pos = 0;
                element.cursor_state.reset_blink(Instant::now());
                let on_change = element.on_change.clone();
                if !old_value.is_empty() {
                    if let Some(f) = on_change {
                        f("");
                    }
                }
                self.needs_relayout = true;
            }
        }
    }

    /// Press a key (or key combo) on the element matching `selector`.
    ///
    /// Supports single keys (`"Enter"`, `"Backspace"`, `"a"`) and combos
    /// with modifiers like `"Ctrl+A"`. Currently, the Ctrl modifier is
    /// handled specially for `Ctrl+A` (select all / clear).
    ///
    /// The element is focused (clicked) before the key is pressed.
    pub fn press_on(&mut self, selector: &str, key_str: &str) {
        self.trace
            .record(TraceAction::Press { selector: selector.to_owned(), key: key_str.to_owned() });
        let (cx, cy) = self.resolve_center(selector);
        self.click(cx, cy);
        self.press_key_str(key_str);
        self.step();
    }

    /// Parse a key string like "Enter", "Backspace", "Ctrl+A" and dispatch it.
    pub(crate) fn press_key_str(&mut self, key_str: &str) {
        let parts: Vec<&str> = key_str.split('+').collect();
        let has_ctrl = parts.iter().any(|p| p.eq_ignore_ascii_case("ctrl"));

        let key_part = parts.last().unwrap_or(&"");

        if has_ctrl && key_part.eq_ignore_ascii_case("a") {
            let focused = self.interaction.focused;
            self.clear_input(focused);
            return;
        }

        let key = parse_key(key_part);
        self.press_key(key);
    }

    /// Select an option by value on the first `<select>` matching `selector`.
    pub fn select_option_on(&mut self, selector: &str, value: &str) {
        self.trace.record(TraceAction::SelectOption {
            selector: selector.to_owned(),
            value: value.to_owned(),
        });
        let node_id = self.resolve(selector).node_id;
        let index = {
            let ss = self.select_state(node_id).unwrap_or_else(|| {
                panic!("select_option_on: element matching '{}' is not a <select>", selector)
            });
            ss.options.iter().position(|o| o.value == value).unwrap_or_else(|| {
                panic!(
                    "select_option_on: no option with value '{}' in select '{}'",
                    value, selector
                )
            })
        };
        self.select_choose(node_id, index as u32);
        self.step();
    }

    /// Select an option by index on the first `<select>` matching `selector`.
    pub fn select_option_by_index_on(&mut self, selector: &str, index: usize) {
        let node_id = self.resolve(selector).node_id;
        let _ = self.select_state(node_id).unwrap_or_else(|| {
            panic!("select_option_by_index_on: element matching '{}' is not a <select>", selector)
        });
        self.select_choose(node_id, index as u32);
        self.step();
    }
}

/// Parse a key name string into a `Key` enum value.
fn parse_key(s: &str) -> Key {
    Key::from_name(s).unwrap_or_else(|| {
        // Single character keys
        let mut chars = s.chars();
        if let Some(ch) = chars.next() {
            if chars.next().is_none() {
                return Key::Char(ch);
            }
        }
        panic!("press_on: unknown key '{}'", s)
    })
}
