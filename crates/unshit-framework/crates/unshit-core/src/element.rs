use crate::cell_grid::CellGrid;
use crate::cursor::CursorState;
use crate::dirty::DirtyFlags;
use crate::id::{NodeId, NodeRef};
use crate::resize_handle::{PaneResizeEvent, ResizeAxis};
use crate::style::parse::StyleDeclaration;
use crate::style::transition::RunningTransition;
use crate::style::types::{ComputedStyle, SelectionStyle};
use crate::svg::types::SvgNode;
use smallvec::SmallVec;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    Div,
    Span,
    Text,
    Button,
    Input,
    Canvas,
    Svg,
    SvgPath,
    SvgCircle,
    SvgRect,
    SvgLine,
    SvgPolyline,
    SvgPolygon,
    SvgGroup,
    Select,
    Option,
}

impl Tag {
    pub fn name(&self) -> &'static str {
        match self {
            Tag::Div => "div",
            Tag::Span => "span",
            Tag::Text => "text",
            Tag::Button => "button",
            Tag::Input => "input",
            Tag::Canvas => "canvas",
            Tag::Svg => "svg",
            Tag::SvgPath => "path",
            Tag::SvgCircle => "circle",
            Tag::SvgRect => "rect",
            Tag::SvgLine => "line",
            Tag::SvgPolyline => "polyline",
            Tag::SvgPolygon => "polygon",
            Tag::SvgGroup => "g",
            Tag::Select => "select",
            Tag::Option => "option",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Tag> {
        match s {
            "div" => Some(Tag::Div),
            "span" => Some(Tag::Span),
            "text" => Some(Tag::Text),
            "button" => Some(Tag::Button),
            "input" => Some(Tag::Input),
            "canvas" => Some(Tag::Canvas),
            "svg" => Some(Tag::Svg),
            "path" => Some(Tag::SvgPath),
            "circle" => Some(Tag::SvgCircle),
            "rect" => Some(Tag::SvgRect),
            "line" => Some(Tag::SvgLine),
            "polyline" => Some(Tag::SvgPolyline),
            "polygon" => Some(Tag::SvgPolygon),
            "g" => Some(Tag::SvgGroup),
            "select" => Some(Tag::Select),
            "option" => Some(Tag::Option),
            _ => None,
        }
    }

    /// Returns true if this tag is any of the SVG variants. Used by the
    /// renderer to route elements through the SVG tessellation path.
    pub fn is_svg(self) -> bool {
        matches!(
            self,
            Tag::Svg
                | Tag::SvgPath
                | Tag::SvgCircle
                | Tag::SvgRect
                | Tag::SvgLine
                | Tag::SvgPolyline
                | Tag::SvgPolygon
                | Tag::SvgGroup
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ElementContent {
    None,
    Text(String),
    Image(String),
    Canvas,
    Grid(CellGrid),
    Svg(SvgNode),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LayoutRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl LayoutRect {
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.width && py >= self.y && py <= self.y + self.height
    }
}

/// The type of an input element, mirroring the HTML `type` attribute.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum InputType {
    /// Plain text input (default).
    #[default]
    Text,
    /// Password input: renders value as bullet characters.
    Password,
    /// Checkbox: toggleable boolean widget.
    Checkbox,
    /// Radio button: single-select within a named group.
    Radio,
    /// Numeric text input: filters to digits, sign, and decimal point.
    Number,
    /// Range slider: thumb on a horizontal track.
    Range,
    /// Hidden input: no layout or rendering.
    Hidden,
}

impl InputType {
    pub fn from_str(s: &str) -> Self {
        match s {
            "password" => InputType::Password,
            "checkbox" => InputType::Checkbox,
            "radio" => InputType::Radio,
            "number" => InputType::Number,
            "range" => InputType::Range,
            "hidden" => InputType::Hidden,
            _ => InputType::Text,
        }
    }
}

/// Runtime state for text input elements.
/// Stored on Element directly and preserved across reconciliation.
#[derive(Clone, Debug)]
pub struct InputState {
    pub value: String,
    pub cursor_pos: usize, // byte offset into value
    pub input_type: InputType,
    /// For checkbox/radio: whether the control is checked.
    pub checked: bool,
    /// For number/range: the parsed numeric value.
    pub numeric_value: f32,
    /// Minimum value for number/range inputs.
    pub min: f32,
    /// Maximum value for number/range inputs.
    pub max: f32,
    /// Step increment for number/range inputs.
    pub step: f32,
    /// In-progress IME composition string (preedit). None when no composition is active.
    pub preedit: Option<String>,
    /// Byte-indexed cursor range within the preedit string. None means the cursor is hidden.
    pub preedit_cursor: Option<(usize, usize)>,
}

impl Default for InputState {
    fn default() -> Self {
        Self {
            value: String::new(),
            cursor_pos: 0,
            input_type: InputType::Text,
            checked: false,
            numeric_value: 0.0,
            min: 0.0,
            max: 100.0,
            step: 1.0,
            preedit: None,
            preedit_cursor: None,
        }
    }
}

/// A single option entry in a select widget.
#[derive(Clone, Debug, PartialEq)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
}

/// Runtime state for select elements.
/// Stored per-node on Tag::Select elements only.
#[derive(Clone, Debug, Default)]
pub struct SelectState {
    pub open: bool,
    pub selected_index: u32,
    pub highlighted_index: Option<u32>,
    pub options: Vec<SelectOption>,
}

pub type EventHandler =
    Arc<dyn Fn(&crate::event::Event) -> Option<Box<dyn std::any::Any>> + Send + Sync>;

pub struct Element {
    // Tree links
    pub parent: NodeId,
    pub first_child: NodeId,
    pub last_child: NodeId,
    pub next_sibling: NodeId,
    pub prev_sibling: NodeId,

    // Identity
    pub tag: Tag,
    pub id: Option<String>,
    pub key: Option<String>,
    pub classes: SmallVec<[String; 4]>,

    // Style
    pub computed_style: ComputedStyle,

    // Layout
    pub taffy_node: Option<taffy::NodeId>,
    pub layout_rect: LayoutRect,

    // Scroll state
    pub scroll_x: f32,
    pub scroll_y: f32,

    // Dirty tracking
    pub dirty: DirtyFlags,

    // Content
    pub content: ElementContent,

    // Focus
    pub tab_index: Option<i32>,

    // Keyboard capture
    pub captures_keyboard: bool,

    // Event handlers
    pub handlers: SmallVec<[(crate::event::EventType, EventHandler); 2]>,
    pub on_click: Option<Arc<dyn Fn() + Send + Sync>>,
    pub on_context_menu: Option<Arc<dyn Fn() + Send + Sync>>,
    pub on_drag: Option<Arc<dyn Fn(&crate::event::DragEvent) + Send + Sync>>,
    pub on_resize: Option<Arc<dyn Fn(f32, f32) + Send + Sync>>,

    // Previous layout dimensions (for resize detection)
    pub prev_width: f32,
    pub prev_height: f32,

    // Resize handle
    pub resize_axis: Option<ResizeAxis>,
    pub on_pane_resize: Option<Arc<dyn Fn(&PaneResizeEvent) + Send + Sync>>,

    // Text input
    pub input_state: InputState,
    pub placeholder: Option<String>,
    pub on_change: Option<Arc<dyn Fn(&str) + Send + Sync>>,
    pub on_submit: Option<Arc<dyn Fn(&str) + Send + Sync>>,
    /// Name attribute (used for radio grouping).
    pub name: Option<String>,

    // Select widget state (only populated for Tag::Select nodes)
    pub select_state: Option<SelectState>,

    // Lifecycle hooks
    pub on_mount: Option<Arc<dyn Fn(NodeId) + Send + Sync>>,
    pub on_unmount: Option<Arc<dyn Fn(NodeId) + Send + Sync>>,

    // Transitions
    pub running_transitions: SmallVec<[RunningTransition; 4]>,
    pub previous_style: Option<Box<ComputedStyle>>,

    // Persistent buffer rendering opt-in.
    // When true, the renderer maintains a GPU-side instance buffer for this
    // element that survives across frames and is updated incrementally via
    // damage regions rather than rebuilt from scratch every frame.
    pub persistent_buffer: bool,

    // Cursor blink state (per-element)
    pub cursor_state: CursorState,

    pub selection_style: Option<SelectionStyle>,

    // Synthetic pseudo element marker. When true this element was allocated
    // by the pseudo element resolver (for example a ::before or ::after
    // synthesized child) and must not participate in user tree reconciliation.
    pub synthetic: bool,

    // Memo key for subtree memoization. When present and matching the new
    // definition's memo_key, the entire subtree is skipped during reconciliation.
    pub memo_key: Option<u64>,

    // Optional ref handle. When set, the reconciler writes the NodeId into this
    // handle after mounting and clears it before unmounting.
    pub node_ref: Option<NodeRef>,

    /// Inline style overrides applied after the CSS cascade. These take
    /// highest precedence, equivalent to HTML `style="..."` attributes.
    pub style_overrides: SmallVec<[StyleDeclaration; 2]>,
}

impl Element {
    pub fn new(tag: Tag) -> Self {
        Self {
            parent: NodeId::DANGLING,
            first_child: NodeId::DANGLING,
            last_child: NodeId::DANGLING,
            next_sibling: NodeId::DANGLING,
            prev_sibling: NodeId::DANGLING,
            tag,
            id: None,
            key: None,
            classes: SmallVec::new(),
            computed_style: ComputedStyle::default(),
            taffy_node: None,
            layout_rect: LayoutRect::default(),
            scroll_x: 0.0,
            scroll_y: 0.0,
            dirty: DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT | DirtyFlags::CHILDREN,
            content: ElementContent::None,
            tab_index: None,
            captures_keyboard: false,
            handlers: SmallVec::new(),
            on_click: None,
            on_context_menu: None,
            on_drag: None,
            on_resize: None,
            prev_width: 0.0,
            prev_height: 0.0,
            resize_axis: None,
            on_pane_resize: None,
            input_state: InputState::default(),
            placeholder: None,
            on_change: None,
            on_submit: None,
            name: None,
            select_state: None,
            on_mount: None,
            on_unmount: None,
            running_transitions: SmallVec::new(),
            previous_style: None,
            persistent_buffer: false,
            cursor_state: CursorState::default(),
            selection_style: None,
            synthetic: false,
            memo_key: None,
            node_ref: None,
            style_overrides: SmallVec::new(),
        }
    }

    pub fn tag_name(&self) -> &str {
        self.tag.name()
    }

    /// Returns true if this element was allocated by the pseudo element
    /// resolver. Synthetic elements are skipped by the reconciler so they do
    /// not get confused with user tree children.
    pub fn is_synthetic(&self) -> bool {
        self.synthetic
    }

    pub fn has_children(&self) -> bool {
        !self.first_child.is_dangling()
    }

    /// An element is focusable if it is a Button/Input (except hidden)/Select or has an explicit tab_index.
    pub fn is_focusable(&self) -> bool {
        let input_focusable =
            self.tag == Tag::Input && self.input_state.input_type != InputType::Hidden;
        matches!(self.tag, Tag::Button | Tag::Select) || input_focusable || self.tab_index.is_some()
    }

    /// Compare and update this element's mutable fields from an `ElementDef`.
    /// Returns `DirtyFlags` indicating what changed.
    ///
    /// This must only be called when `def.tag == self.tag`. A tag mismatch
    /// means the reconciler should replace the node entirely, but we return
    /// all flags as a safety fallback.
    pub fn update_from_def(&mut self, def: &ElementDef) -> DirtyFlags {
        if def.tag != self.tag {
            return DirtyFlags::STYLE
                | DirtyFlags::LAYOUT
                | DirtyFlags::PAINT
                | DirtyFlags::CHILDREN;
        }

        let mut flags = DirtyFlags::empty();

        if def.id != self.id {
            self.id = def.id.clone();
            flags |= DirtyFlags::STYLE;
        }

        if def.classes[..] != self.classes[..] {
            self.classes = def.classes.clone();
            flags |= DirtyFlags::STYLE;
        }

        if def.content != self.content {
            self.content = def.content.clone();
            flags |= DirtyFlags::LAYOUT;
        }

        if def.tab_index != self.tab_index {
            self.tab_index = def.tab_index;
        }

        if def.captures_keyboard != self.captures_keyboard {
            self.captures_keyboard = def.captures_keyboard;
        }

        // Closures are Arc<dyn Fn>, not comparable; always replace.
        self.on_click = def.on_click.clone();
        self.on_context_menu = def.on_context_menu.clone();
        self.on_drag = def.on_drag.clone();
        self.on_resize = def.on_resize.clone();
        self.resize_axis = def.resize_axis;
        self.on_pane_resize = def.on_pane_resize.clone();
        self.placeholder = def.placeholder.clone();
        self.on_change = def.on_change.clone();
        self.on_submit = def.on_submit.clone();
        self.persistent_buffer = def.persistent_buffer;
        self.name = def.name.clone();
        // Update input type from def; preserve user state (checked, numeric_value).
        if self.input_state.input_type != def.input_type {
            self.input_state.input_type = def.input_type;
        }
        // Update range bounds from def (overrides defaults, but never shrinks user-entered value).
        if let Some(min) = def.min {
            self.input_state.min = min;
        }
        if let Some(max) = def.max {
            self.input_state.max = max;
        }
        if let Some(step) = def.step {
            self.input_state.step = step;
        }
        // Note: input_state.value, cursor_pos, checked, numeric_value are NOT
        // updated here (preserved like scroll state).

        // Apply inline style overrides. Since StyleDeclaration does not derive
        // PartialEq, mark LAYOUT dirty whenever either side is non-empty.
        let overrides_changed =
            !self.style_overrides.is_empty() || !def.style_overrides.is_empty();
        self.style_overrides = def.style_overrides.clone();
        if overrides_changed {
            flags |= DirtyFlags::LAYOUT;
        }

        // For select elements: update options list but preserve open/highlighted state.
        if self.tag == Tag::Select {
            let new_opts: Vec<SelectOption> = def
                .options
                .iter()
                .map(|(v, l)| SelectOption { value: v.clone(), label: l.clone() })
                .collect();
            if let Some(ref mut ss) = self.select_state {
                if ss.options != new_opts {
                    ss.options = new_opts;
                    flags |= DirtyFlags::PAINT;
                }
            }
        }

        flags
    }
}

pub struct ElementDef {
    pub tag: Tag,
    pub id: Option<String>,
    pub key: Option<String>,
    pub classes: SmallVec<[String; 4]>,
    pub content: ElementContent,
    pub children: Vec<ElementDef>,
    pub on_click: Option<Arc<dyn Fn() + Send + Sync>>,
    pub tab_index: Option<i32>,
    pub captures_keyboard: bool,
    pub on_context_menu: Option<Arc<dyn Fn() + Send + Sync>>,
    pub on_drag: Option<Arc<dyn Fn(&crate::event::DragEvent) + Send + Sync>>,
    pub on_resize: Option<Arc<dyn Fn(f32, f32) + Send + Sync>>,
    pub handlers: SmallVec<[(crate::event::EventType, EventHandler); 2]>,
    pub resize_axis: Option<ResizeAxis>,
    pub on_pane_resize: Option<Arc<dyn Fn(&PaneResizeEvent) + Send + Sync>>,
    pub placeholder: Option<String>,
    pub on_change: Option<Arc<dyn Fn(&str) + Send + Sync>>,
    pub on_submit: Option<Arc<dyn Fn(&str) + Send + Sync>>,
    pub persistent_buffer: bool,
    /// Memo key for subtree memoization. When provided and equal to the
    /// live element's memo_key, the reconciler skips this entire subtree.
    pub memo_key: Option<u64>,
    // Input type attributes
    pub input_type: InputType,
    pub checked: bool,
    pub min: Option<f32>,
    pub max: Option<f32>,
    pub step: Option<f32>,
    /// Name used for radio grouping.
    pub name: Option<String>,
    // Select widget options: (value, label) pairs, used only for Tag::Select defs
    pub options: Vec<(String, String)>,
    pub selected_index: Option<u32>,
    // Lifecycle hooks
    pub on_mount: Option<Arc<dyn Fn(NodeId) + Send + Sync>>,
    pub on_unmount: Option<Arc<dyn Fn(NodeId) + Send + Sync>>,
    /// Optional ref handle. When set, the reconciler writes the allocated
    /// `NodeId` into this handle after mounting and clears it on unmount.
    pub node_ref: Option<NodeRef>,
    /// Inline style overrides applied after the CSS cascade.
    pub style_overrides: SmallVec<[StyleDeclaration; 2]>,
}

impl ElementDef {
    pub fn new(tag: Tag) -> Self {
        Self {
            tag,
            id: None,
            key: None,
            classes: SmallVec::new(),
            content: ElementContent::None,
            children: Vec::new(),
            on_click: None,
            tab_index: None,
            captures_keyboard: false,
            on_context_menu: None,
            on_drag: None,
            on_resize: None,
            handlers: SmallVec::new(),
            resize_axis: None,
            on_pane_resize: None,
            placeholder: None,
            on_change: None,
            on_submit: None,
            persistent_buffer: false,
            memo_key: None,
            input_type: InputType::Text,
            checked: false,
            min: None,
            max: None,
            step: None,
            name: None,
            options: Vec::new(),
            selected_index: None,
            on_mount: None,
            on_unmount: None,
            node_ref: None,
            style_overrides: SmallVec::new(),
        }
    }

    pub fn on_click(mut self, f: impl Fn() + Send + Sync + 'static) -> Self {
        self.on_click = Some(Arc::new(f));
        self
    }

    pub fn on_context_menu(mut self, f: impl Fn() + Send + Sync + 'static) -> Self {
        self.on_context_menu = Some(Arc::new(f));
        self
    }

    pub fn on_drag(mut self, f: impl Fn(&crate::event::DragEvent) + Send + Sync + 'static) -> Self {
        self.on_drag = Some(Arc::new(f));
        self
    }

    pub fn on_resize(mut self, f: impl Fn(f32, f32) + Send + Sync + 'static) -> Self {
        self.on_resize = Some(Arc::new(f));
        self
    }

    /// Register a generic event handler for the given event type.
    pub fn on(
        mut self,
        event_type: crate::event::EventType,
        f: impl Fn(&crate::event::Event) -> Option<Box<dyn std::any::Any>> + Send + Sync + 'static,
    ) -> Self {
        self.handlers.push((event_type, Arc::new(f)));
        self
    }

    pub fn with_resize_axis(mut self, axis: ResizeAxis) -> Self {
        self.resize_axis = Some(axis);
        self
    }

    pub fn on_pane_resize(mut self, f: impl Fn(&PaneResizeEvent) + Send + Sync + 'static) -> Self {
        self.on_pane_resize = Some(Arc::new(f));
        self
    }

    pub fn with_class(mut self, class: impl Into<String>) -> Self {
        self.classes.push(class.into());
        self
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.content = ElementContent::Text(text.into());
        self
    }

    pub fn with_child(mut self, child: ElementDef) -> Self {
        self.children.push(child);
        self
    }

    pub fn with_image(mut self, path: impl Into<String>) -> Self {
        self.content = ElementContent::Image(path.into());
        self
    }

    pub fn with_children(mut self, children: Vec<ElementDef>) -> Self {
        self.children = children;
        self
    }

    pub fn with_tab_index(mut self, index: i32) -> Self {
        self.tab_index = Some(index);
        self
    }

    pub fn captures_keyboard(mut self, v: bool) -> Self {
        self.captures_keyboard = v;
        self
    }

    pub fn with_canvas(mut self) -> Self {
        self.tag = Tag::Canvas;
        self.content = ElementContent::Canvas;
        self
    }

    pub fn with_grid(mut self, grid: CellGrid) -> Self {
        self.content = ElementContent::Grid(grid);
        self
    }

    /// Attach an inline SVG subtree to this element. The wrapping `ElementDef`
    /// becomes an `<svg>` root; the provided `SvgNode` holds the parsed
    /// primitives and cascaded attributes.
    pub fn with_svg(mut self, node: SvgNode) -> Self {
        self.tag = Tag::Svg;
        self.content = ElementContent::Svg(node);
        self
    }

    pub fn with_placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = Some(text.into());
        self
    }

    pub fn on_change(mut self, f: impl Fn(&str) + Send + Sync + 'static) -> Self {
        self.on_change = Some(Arc::new(f));
        self
    }

    pub fn on_submit(mut self, f: impl Fn(&str) + Send + Sync + 'static) -> Self {
        self.on_submit = Some(Arc::new(f));
        self
    }

    pub fn with_persistent_buffer(mut self, enabled: bool) -> Self {
        self.persistent_buffer = enabled;
        self
    }

    /// Set a memo key for this subtree. When present and matching the live
    /// element's memo_key, the reconciler will skip this entire subtree,
    /// reusing the existing children without diffing them.
    pub fn with_memo_key(mut self, key: u64) -> Self {
        self.memo_key = Some(key);
        self
    }

    pub fn with_input_type(mut self, input_type: InputType) -> Self {
        self.input_type = input_type;
        self
    }

    pub fn with_checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn with_min(mut self, min: f32) -> Self {
        self.min = Some(min);
        self
    }

    pub fn with_max(mut self, max: f32) -> Self {
        self.max = Some(max);
        self
    }

    pub fn with_step(mut self, step: f32) -> Self {
        self.step = Some(step);
        self
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the option list for a `Tag::Select` element.
    /// Each entry is `(value, label)`.
    pub fn with_options(mut self, opts: Vec<(String, String)>) -> Self {
        self.options = opts;
        self
    }

    /// Set the initially selected index for a `Tag::Select` element.
    pub fn with_selected_index(mut self, index: u32) -> Self {
        self.selected_index = Some(index);
        self
    }

    /// Register a callback fired once when this node is first mounted into the tree.
    pub fn on_mount(mut self, f: impl Fn(NodeId) + Send + Sync + 'static) -> Self {
        self.on_mount = Some(Arc::new(f));
        self
    }

    /// Register a callback fired once when this node is removed from the tree.
    pub fn on_unmount(mut self, f: impl Fn(NodeId) + Send + Sync + 'static) -> Self {
        self.on_unmount = Some(Arc::new(f));
        self
    }

    /// Attach a `NodeRef` handle. The reconciler will store the allocated
    /// `NodeId` into `node_ref` after mounting and clear it before unmounting.
    pub fn with_ref(mut self, node_ref: NodeRef) -> Self {
        self.node_ref = Some(node_ref);
        self
    }

    /// Add an inline style override that takes precedence over CSS cascade.
    pub fn with_style(mut self, decl: StyleDeclaration) -> Self {
        self.style_overrides.push(decl);
        self
    }
}

pub struct ElementTree {
    pub root: ElementDef,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_element_content_eq_none() {
        assert_eq!(ElementContent::None, ElementContent::None);
    }

    #[test]
    fn test_element_content_eq_text() {
        assert_eq!(ElementContent::Text("a".into()), ElementContent::Text("a".into()));
        assert_ne!(ElementContent::Text("a".into()), ElementContent::Text("b".into()));
    }

    #[test]
    fn test_element_content_eq_text_vs_none() {
        assert_ne!(ElementContent::Text("a".into()), ElementContent::None);
    }

    #[test]
    fn test_element_content_eq_image() {
        assert_eq!(ElementContent::Image("x.png".into()), ElementContent::Image("x.png".into()));
    }

    #[test]
    fn test_update_from_def_no_change() {
        let mut elem = Element::new(Tag::Div);
        elem.dirty = DirtyFlags::empty();

        let def = ElementDef::new(Tag::Div);
        let flags = elem.update_from_def(&def);

        assert!(flags.is_empty());
    }

    #[test]
    fn test_update_from_def_class_change() {
        let mut elem = Element::new(Tag::Div);
        elem.dirty = DirtyFlags::empty();

        let def = ElementDef::new(Tag::Div).with_class("highlight");
        let flags = elem.update_from_def(&def);

        assert!(flags.contains(DirtyFlags::STYLE));
        assert_eq!(elem.classes[..], def.classes[..]);
    }

    #[test]
    fn test_update_from_def_content_change() {
        let mut elem = Element::new(Tag::Div);
        elem.dirty = DirtyFlags::empty();

        let def = ElementDef::new(Tag::Div).with_text("hello");
        let flags = elem.update_from_def(&def);

        assert!(flags.contains(DirtyFlags::LAYOUT));
        assert_eq!(elem.content, ElementContent::Text("hello".into()));
    }

    #[test]
    fn test_update_from_def_id_change() {
        let mut elem = Element::new(Tag::Div);
        elem.dirty = DirtyFlags::empty();

        let def = ElementDef::new(Tag::Div).with_id("main");
        let flags = elem.update_from_def(&def);

        assert!(flags.contains(DirtyFlags::STYLE));
        assert_eq!(elem.id, Some("main".into()));
    }

    #[test]
    fn test_update_from_def_preserves_scroll() {
        let mut elem = Element::new(Tag::Div);
        elem.scroll_x = 42.0;
        elem.scroll_y = 99.0;
        elem.dirty = DirtyFlags::empty();

        let def = ElementDef::new(Tag::Div).with_text("changed");
        elem.update_from_def(&def);

        assert_eq!(elem.scroll_x, 42.0);
        assert_eq!(elem.scroll_y, 99.0);
    }

    #[test]
    fn test_tag_from_str_svg_variants() {
        assert_eq!(Tag::from_str("svg"), Some(Tag::Svg));
        assert_eq!(Tag::from_str("path"), Some(Tag::SvgPath));
        assert_eq!(Tag::from_str("circle"), Some(Tag::SvgCircle));
        assert_eq!(Tag::from_str("rect"), Some(Tag::SvgRect));
        assert_eq!(Tag::from_str("line"), Some(Tag::SvgLine));
        assert_eq!(Tag::from_str("polyline"), Some(Tag::SvgPolyline));
        assert_eq!(Tag::from_str("polygon"), Some(Tag::SvgPolygon));
        assert_eq!(Tag::from_str("g"), Some(Tag::SvgGroup));
    }

    #[test]
    fn test_tag_name_svg_variants() {
        assert_eq!(Tag::Svg.name(), "svg");
        assert_eq!(Tag::SvgPath.name(), "path");
        assert_eq!(Tag::SvgCircle.name(), "circle");
        assert_eq!(Tag::SvgGroup.name(), "g");
    }

    #[test]
    fn test_tag_is_svg_flag() {
        assert!(Tag::Svg.is_svg());
        assert!(Tag::SvgPath.is_svg());
        assert!(!Tag::Div.is_svg());
        assert!(!Tag::Canvas.is_svg());
    }

    #[test]
    fn test_with_svg_wraps_svg_node() {
        use crate::svg::types::{SvgNode, SvgPrimitive};
        let node = SvgNode {
            primitive: SvgPrimitive::Circle { cx: 5.0, cy: 5.0, r: 2.0 },
            attrs: Default::default(),
            children: Vec::new(),
        };
        let def = ElementDef::new(Tag::Div).with_svg(node.clone());
        assert_eq!(def.tag, Tag::Svg);
        assert_eq!(def.content, ElementContent::Svg(node));
    }

    #[test]
    fn test_update_from_def_preserves_layout_rect() {
        let mut elem = Element::new(Tag::Div);
        elem.layout_rect = LayoutRect { x: 10.0, y: 20.0, width: 100.0, height: 50.0 };
        elem.dirty = DirtyFlags::empty();

        let def = ElementDef::new(Tag::Div).with_class("new-class");
        elem.update_from_def(&def);

        assert_eq!(elem.layout_rect.x, 10.0);
        assert_eq!(elem.layout_rect.y, 20.0);
        assert_eq!(elem.layout_rect.width, 100.0);
        assert_eq!(elem.layout_rect.height, 50.0);
    }

    #[test]
    fn test_input_state_preedit_defaults_to_none() {
        let state = InputState::default();
        assert!(state.preedit.is_none(), "preedit must be None by default");
        assert!(state.preedit_cursor.is_none(), "preedit_cursor must be None by default");
    }

    #[test]
    fn test_input_state_preedit_composition_cycle() {
        // Simulate a full IME composition cycle: set preedit, update cursor, then commit.
        let mut state = InputState::default();
        state.value = "Hello".to_string();
        state.cursor_pos = 5;

        // Preedit phase: in-progress composition
        state.preedit = Some("世界".to_string());
        state.preedit_cursor = Some((0, 6)); // 6 bytes for two CJK chars

        assert_eq!(state.preedit.as_deref(), Some("世界"));
        assert_eq!(state.preedit_cursor, Some((0, 6)));

        // Commit phase: insert committed text, clear preedit
        let committed = "世界".to_string();
        state.value.insert_str(state.cursor_pos, &committed);
        state.cursor_pos += committed.len();
        state.preedit = None;
        state.preedit_cursor = None;

        assert_eq!(state.value, "Hello世界");
        assert_eq!(state.cursor_pos, 11); // 5 (Hello) + 6 (世界 in UTF-8)
        assert!(state.preedit.is_none(), "preedit must be cleared after commit");
        assert!(state.preedit_cursor.is_none(), "preedit_cursor must be cleared after commit");
    }

    #[test]
    fn test_input_state_preedit_clear_on_disabled() {
        // Simulate IME disabled: any pending preedit must be cleared.
        let mut state = InputState::default();
        state.preedit = Some("あ".to_string());
        state.preedit_cursor = Some((0, 3));

        // Disabled: clear preedit fields
        state.preedit = None;
        state.preedit_cursor = None;

        assert!(state.preedit.is_none());
        assert!(state.preedit_cursor.is_none());
    }
}
