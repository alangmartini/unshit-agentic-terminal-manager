pub mod build;
pub mod cell_grid;
pub mod cursor;
pub mod damage;
pub mod dirty;
pub mod element;
pub mod event;
pub mod frame_arena;
pub mod grid_font;
pub mod id;
pub mod input;
pub mod layout;
pub mod reconcile;
pub mod resize_handle;
pub mod scroll;
pub mod shortcut;
pub mod style;
pub mod svg;
pub mod toast;
pub mod trace;
pub mod tree;

pub use toast::{Toast, ToastId, ToastKind, ToastStore};

pub mod prelude {
    pub use crate::cell_grid::{Cell, CellAttrs, CellGrid};
    pub use crate::cursor::{CursorShape, CursorState};
    pub use crate::dirty::DirtyFlags;
    pub use crate::element::*;
    pub use crate::event::*;
    pub use crate::grid_font::GridFont;
    pub use crate::id::{NodeId, NodeRef};
    pub use crate::shortcut::{KeyCombo, Shortcut, ShortcutRegistry};
    pub use crate::style::motion::{Easing, Spring, Tween};
    pub use crate::style::parse::CompiledStylesheet;
    pub use crate::style::theme::{Theme, ThemeColors, ThemeMotion};
    pub use crate::style::types::*;
    pub use crate::svg::{
        PathCommand, StrokeLineCap, StrokeLineJoin, SvgAttrs, SvgNode, SvgPaint, SvgPrimitive,
        SvgTransform, ViewBox,
    };
    pub use crate::tree::NodeArena;
}
