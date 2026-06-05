# Command Palette Redesign Todo

- [x] Task 1: Add Palette Result Model
  - Acceptance: modes parse, fuzzy ranks, seven safe actions exist, real result builder emits no prototype data.
  - Verify: `cargo test -p terminal-manager command_palette`
  - Files: `src/command_palette.rs`, `src/main.rs`

- [x] Task 2: Make Ctrl+Shift+P The Editable Default
  - Acceptance: editable default is `Ctrl+Shift+P`, settings still resolves overrides, `Ctrl+K` alias does not conflict.
  - Verify: `cargo test -p terminal-manager keybind`
  - Files: `src/keybinds/mod.rs`, `src/keybinds/registry.rs`, `src/keybinds/state.rs`

- [x] Checkpoint: Foundation
  - Verify: `cargo test -p terminal-manager command_palette`
  - Verify: `cargo test -p terminal-manager keybind`

- [x] Task 3: Add Palette State Selection And Safe Execution
  - Acceptance: open/close/query reset active state, selection wraps, escape clears then closes, active/explicit execution dispatches safe actions and real navigation only.
  - Verify: `cargo test -p terminal-manager palette`
  - Files: `src/state.rs`, `src/command_palette.rs`

- [x] Task 4: Wire Keyboard Navigation Without Breaking Terminal Input
  - Acceptance: palette handles ArrowUp/Down, Ctrl+P/N, Enter, Escape only while open; terminal input unaffected when closed.
  - Verify: `cargo test -p terminal-manager palette`; if framework touched, `cargo test -p unshit-app keyboard`
  - Files: `src/main.rs`, `src/state.rs`, `src/keybinds/registry.rs`, optional `crates/unshit-framework/crates/unshit-app/src/app.rs`

- [x] Checkpoint: Behavior
  - Verify: safe commands execute through existing dispatch.
  - Verify: no synchronous PTY IPC added.

- [x] Task 5: Render Handoff-Style Grouped Palette UI
  - Acceptance: handoff shell renders, mode chip/pills render, active row follows state, click/submit execute.
  - Verify: `cargo test -p terminal-manager command_palette`
  - Files: `src/ui/command_palette.rs`, `src/command_palette.rs`, `src/state.rs`

- [x] Task 6: Implement Real Mode Content And Honest Empty States
  - Acceptance: `>`, unified, and `:` show real executable rows; `@` and `/` show honest empty states when no real source exists.
  - Verify: `cargo test -p terminal-manager command_palette`; `cargo test -p terminal-manager terminal_focus`; `cargo fmt --check`
  - Files: `src/command_palette.rs`, `src/ui/command_palette.rs`, `src/state.rs`

- [x] Task 7: Apply Handoff CSS And Visual Regression Coverage
  - Acceptance: scrim, panel, active rail, preview, footer, chips, empty state, and responsive constraints match handoff intent.
  - Verify: `cargo test -p terminal-manager command_palette`; `cargo fmt --check`; `cargo test -p terminal-manager`; `cargo clippy -p terminal-manager -- -D warnings`
  - Manual run: not practical in this non-interactive worker because `cargo run` opens a native window that cannot be inspected or controlled from the shell without leaving the app running. Covered with CSS contract assertions for layout, preview, footer, chips, empty state, and narrow viewport rules.
  - Files: `assets/styles.css`, `src/ui/command_palette.rs`, optional `src/main.rs`

- [x] Checkpoint: Complete
  - Verify: all `SPEC.md` success criteria met or documented as unavailable real data.
  - Verify: no fake prototype rows.
  - Verify: `/` scrollback mode remains an honest empty state until a read-only scrollback source exists.
  - Verify: `@` agent mode shows real quick-prompt/agent-terminal state when present, otherwise an honest empty state.
  - Verify: manual `cargo run` checks were not practical in this worker; automated dispatch/UI/style coverage exercises `rename`, `>split`, `@`, `:`, and `/`.
