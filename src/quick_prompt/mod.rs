//! Quick Prompt overlay: press Ctrl+Shift+Q to open a centered card,
//! type a prompt, attach images, and dispatch a Claude or Codex agent
//! into a fresh worktree. Slice 1 lands the empty overlay shell;
//! richer state and behavior land in later slices per `tasks/plan.md`.

pub mod state;
pub mod ui;

pub use state::QuickPromptState;
pub use ui::build_quick_prompt_overlay;
