//! Headless terminal core: cell grid, bounded scrollback, VTE-driven emulator.
//!
//! This crate has no dependency on the rendering stack (wgpu, taffy,
//! unshit-framework). It is shared between the daemon (`unshit-ptyd`) and the
//! UI crate so both can agree on a single on-the-wire representation of a
//! terminal screen plus scrollback.

pub mod cell;
pub mod color;
pub mod grid;
pub mod scrollback;
pub mod snapshot;
pub mod terminal;

pub use cell::{Cell, CellAttrs};
pub use color::{color_256, Color, ANSI_16};
pub use grid::Grid;
pub use scrollback::Scrollback;
pub use snapshot::Snapshot;
pub use terminal::Terminal;
