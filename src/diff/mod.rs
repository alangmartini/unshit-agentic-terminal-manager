//! Unified-diff parsing and the stacked review document the diff pane
//! renders.
//!
//! [`parse`] is pure: `git diff` output goes in, a [`DiffDocument`] comes
//! out — one text line plus one [`DiffRowInfo`] per rendered row, with the
//! file table needed for hunk/file navigation and per-file syntax
//! colouring. [`spec`] turns a user-typed range into validated git
//! arguments, [`job`] runs git off the UI thread, and [`telemetry`] writes
//! the lifecycle records. None of them touch app state.

pub mod job;
pub mod parse;
pub mod spec;
pub mod telemetry;

pub use job::{run_diff, DiffOutcome, MAX_DIFF_STDOUT_BYTES};
pub use parse::{
    parse_unified_diff, DiffDocument, DiffFileInfo, DiffFileStatus, DiffRowInfo, DiffRowKind,
    MAX_DIFF_ROWS,
};
pub use spec::DiffSpec;
