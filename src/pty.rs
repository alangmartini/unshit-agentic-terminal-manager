//! PTY process manager facade for the UI crate.
//!
//! The implementation lives in `unshit-ptyd` so the daemon can own PTYs
//! directly (see SPEC.md section 6, slice 3). This module keeps the old
//! `crate::pty::PtyManager` path working for callers inside the UI crate
//! until slice 3b routes them through the IPC client.

pub use unshit_ptyd::pty::*;
