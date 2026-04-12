use unshit_core::element::{SelectState, Tag};
use unshit_core::id::NodeId;

use crate::TestHarness;

impl TestHarness {
    /// Return the `SelectState` of the given node, if it is a `Tag::Select`.
    pub fn select_state(&self, node_id: NodeId) -> Option<&SelectState> {
        let element = self.arena.get(node_id)?;
        if element.tag != Tag::Select {
            return None;
        }
        element.select_state.as_ref()
    }

    /// Get the index of the currently selected option for a select element.
    pub fn select_selected_index(&self, node_id: NodeId) -> Option<u32> {
        self.select_state(node_id).map(|ss| ss.selected_index)
    }

    /// Get the value of the currently selected option for a select element.
    pub fn select_selected_value(&self, node_id: NodeId) -> Option<String> {
        let ss = self.select_state(node_id)?;
        let idx = ss.selected_index as usize;
        ss.options.get(idx).map(|o| o.value.clone())
    }

    /// Returns `true` if the select's dropdown is open.
    pub fn select_is_open(&self, node_id: NodeId) -> bool {
        self.select_state(node_id).map(|ss| ss.open).unwrap_or(false)
    }

    /// Directly open a select's dropdown (simulates a programmatic toggle).
    pub fn select_open(&mut self, node_id: NodeId) {
        let Some(element) = self.arena.get_mut(node_id) else { return };
        if element.tag != Tag::Select {
            return;
        }
        if let Some(ref mut ss) = element.select_state {
            ss.open = true;
            ss.highlighted_index = Some(ss.selected_index);
        }
    }

    /// Directly close a select's dropdown.
    pub fn select_close(&mut self, node_id: NodeId) {
        let Some(element) = self.arena.get_mut(node_id) else { return };
        if element.tag != Tag::Select {
            return;
        }
        if let Some(ref mut ss) = element.select_state {
            ss.open = false;
            ss.highlighted_index = None;
        }
    }

    /// Directly select an option by index, fires `on_change` if set.
    /// Does NOT require the dropdown to be open.
    pub fn select_choose(&mut self, node_id: NodeId, index: u32) {
        let (value, on_change) = {
            let Some(element) = self.arena.get_mut(node_id) else { return };
            if element.tag != Tag::Select {
                return;
            }
            let (val, cb) = if let Some(ref mut ss) = element.select_state {
                let idx = index.min(ss.options.len().saturating_sub(1) as u32) as usize;
                ss.selected_index = idx as u32;
                ss.open = false;
                ss.highlighted_index = None;
                let v = ss.options.get(idx).map(|o| o.value.clone()).unwrap_or_default();
                (v, element.on_change.clone())
            } else {
                return;
            };
            (val, cb)
        };
        if let Some(f) = on_change {
            f(&value);
        }
    }

    /// Simulate a click on the select element to toggle the dropdown.
    /// This mirrors the app.rs `handle_select_click` logic.
    pub fn click_select(&mut self, node_id: NodeId) {
        // Toggle open
        let Some(element) = self.arena.get_mut(node_id) else { return };
        if element.tag != Tag::Select {
            return;
        }
        if let Some(ref mut ss) = element.select_state {
            ss.open = !ss.open;
            if ss.open {
                ss.highlighted_index = Some(ss.selected_index);
            } else {
                ss.highlighted_index = None;
            }
        }
    }

    /// Simulate clicking a dropdown item by index when the dropdown is open.
    /// Fires `on_change` with the selected value.
    pub fn click_select_option(&mut self, node_id: NodeId, index: usize) {
        self.select_choose(node_id, index as u32);
    }

    /// Move the keyboard highlight within an open select dropdown.
    /// `delta` is +1 for down, -1 for up.
    pub fn select_move_highlight(&mut self, node_id: NodeId, delta: i32) {
        let Some(element) = self.arena.get_mut(node_id) else { return };
        if element.tag != Tag::Select {
            return;
        }
        if let Some(ref mut ss) = element.select_state {
            if ss.options.is_empty() {
                return;
            }
            let len = ss.options.len() as i32;
            let cur = ss.highlighted_index.unwrap_or(ss.selected_index) as i32;
            let next = (cur + delta).clamp(0, len - 1);
            ss.highlighted_index = Some(next as u32);
        }
    }

    /// Confirm the currently highlighted option (Enter/Space press semantics).
    /// Fires `on_change` if the selection changed.
    pub fn select_confirm_highlight(&mut self, node_id: NodeId) {
        self.select_choose_highlighted(node_id);
    }

    fn select_choose_highlighted(&mut self, node_id: NodeId) {
        let (on_change, value) = {
            let Some(element) = self.arena.get_mut(node_id) else { return };
            if element.tag != Tag::Select {
                return;
            }
            if let Some(ref mut ss) = element.select_state {
                let idx = ss.highlighted_index.unwrap_or(ss.selected_index) as usize;
                let idx = idx.min(ss.options.len().saturating_sub(1));
                ss.selected_index = idx as u32;
                ss.open = false;
                ss.highlighted_index = None;
                let val = ss.options.get(idx).map(|o| o.value.clone()).unwrap_or_default();
                (element.on_change.clone(), val)
            } else {
                return;
            }
        };
        if let Some(f) = on_change {
            f(&value);
        }
    }
}
