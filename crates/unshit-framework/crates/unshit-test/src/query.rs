use unshit_core::cursor::CursorState;
use unshit_core::element::{ElementContent, InputType, LayoutRect, Tag};
use unshit_core::id::NodeId;
use unshit_core::style::types::ComputedStyle;

use crate::selector;
use crate::TestHarness;

/// A snapshot of an element's state at the time of the query.
#[derive(Clone, Debug)]
pub struct ElementSnapshot {
    pub node_id: NodeId,
    pub tag: Tag,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub content: ElementContent,
    pub layout_rect: LayoutRect,
    pub computed_style: ComputedStyle,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub input_value: Option<String>,
    pub input_cursor_pos: Option<usize>,
    pub placeholder: Option<String>,
    pub cursor_state: CursorState,
    pub input_type: Option<InputType>,
    pub checked: Option<bool>,
    pub numeric_value: Option<f32>,
}

impl TestHarness {
    /// Find the first element matching a selector string.
    ///
    /// Supported selector forms:
    ///   - Simple: `.classname`, `#id`, `tagname`
    ///   - Compound: `div.active`, `button#submit.primary`
    ///   - Descendant: `.sidebar .menu-item`
    ///   - Child: `.nav > .link`
    ///   - Attribute: `[placeholder="Search"]`, `[type="checkbox"]`
    ///   - Pseudo-class: `:nth-child(2)`, `:first-child`, `:last-child`, `:checked`
    ///   - Text: `text("Click me")`, `has_text("Click")`
    pub fn query(&self, selector: &str) -> Option<ElementSnapshot> {
        let query = selector::parse_query(selector)
            .unwrap_or_else(|e| panic!("invalid selector '{selector}': {e}"));
        let node_id = selector::query_first(&self.arena, self.root, &query)?;
        self.arena.get(node_id).map(|element| snapshot_from(node_id, element))
    }

    /// Find all elements matching a selector string.
    pub fn query_all(&self, selector: &str) -> Vec<ElementSnapshot> {
        let query = selector::parse_query(selector)
            .unwrap_or_else(|e| panic!("invalid selector '{selector}': {e}"));
        selector::query_all(&self.arena, self.root, &query)
            .into_iter()
            .filter_map(|node_id| {
                self.arena.get(node_id).map(|element| snapshot_from(node_id, element))
            })
            .collect()
    }

    /// Get a snapshot of a specific node by its ID.
    pub fn query_node(&self, node_id: NodeId) -> Option<ElementSnapshot> {
        self.arena.get(node_id).map(|element| snapshot_from(node_id, element))
    }
}

pub(crate) fn matches_selector(selector: &str, element: &unshit_core::element::Element) -> bool {
    selector::matches_simple_selector(selector, element)
}

pub(crate) fn snapshot_from(
    node_id: NodeId,
    element: &unshit_core::element::Element,
) -> ElementSnapshot {
    let (input_value, input_cursor_pos, input_type, checked, numeric_value) =
        if element.tag == Tag::Input {
            (
                Some(element.input_state.value.clone()),
                Some(element.input_state.cursor_pos),
                Some(element.input_state.input_type),
                Some(element.input_state.checked),
                Some(element.input_state.numeric_value),
            )
        } else {
            (None, None, None, None, None)
        };

    ElementSnapshot {
        node_id,
        tag: element.tag,
        id: element.id.clone(),
        classes: element.classes.to_vec(),
        content: element.content.clone(),
        layout_rect: element.layout_rect,
        computed_style: element.computed_style.clone(),
        scroll_x: element.scroll_x,
        scroll_y: element.scroll_y,
        input_value,
        input_cursor_pos,
        placeholder: element.placeholder.clone(),
        cursor_state: element.cursor_state.clone(),
        input_type,
        checked,
        numeric_value,
    }
}
