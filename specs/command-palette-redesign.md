# Spec: Command Palette Redesign

## Source Status
Reference URL:
`https://api.anthropic.com/v1/design/h/kua4XnMjzGvMOAA_NZ-4QA?open_file=Command+Palette.html`

Download attempts on 2026-06-04:
- `api.anthropic.com/v1/design/...` returned HTTP 404 `not found`.
- `api.anthropic.com/v1/design/...` without `open_file` returned HTTP 404.
- `claude.ai/design/...` returned HTTP 403.

Exported handoff provided by user on 2026-06-04:
`C:\Users\Alan Beelink\Downloads\command pallete-handoff.zip`

Local extracted reference:
`docs/design/command-palette-handoff/command-pallete/project/Command Palette.html`

Primary files read:
- `docs/design/command-palette-handoff/command-pallete/README.md`
- `docs/design/command-palette-handoff/command-pallete/project/Command Palette.html`
- `docs/design/command-palette-handoff/command-pallete/project/command-palette.jsx`
- `docs/design/command-palette-handoff/command-pallete/project/palette.css`
- `docs/design/command-palette-handoff/command-pallete/project/screenshots/palette-default.png`
- `docs/design/command-palette-handoff/command-pallete/project/screenshots/search-mode.png`

Design source is now available locally.

## Assumptions
1. The target is the exported Claude Design handoff, not the current local palette.
2. The new palette replaces the current `src/ui/command_palette.rs` UI.
3. Existing command dispatch stays Rust-side through `state::dispatch`.
4. `Ctrl+Shift+P` remains the primary opener.
5. Existing rename behavior remains daemon-backed through `dialog.rename_commit`.
6. Prototype fake data must not be shown as real user data; production rows use real app state where available.

## Objective
Build a command palette matching the exported design visually and functionally. It should feel like a terminal-native VS Code-style launcher for terminal-manager actions, sessions, agents, workspace navigation, and scrollback search.

Required first action:
- Rename current terminal.

Expected user flow:
1. User presses `Ctrl+Shift+P`.
2. Palette opens above terminal UI.
3. User filters with fuzzy search.
4. User navigates with arrows or mouse.
5. User presses Enter or clicks a row.
6. `Rename current terminal` opens the existing rename dialog for the active pane.

## Design Requirements
Visual:
- Scrim: dark translucent overlay with blur, anchored near top center.
- Palette shell: 760px target width in compact mode, max viewport constrained, 8px-ish radius, subtle amber glow, border and gradient surface.
- Input row: amber prompt glyph, text input, `esc` key chip.
- Body: scrollable grouped list; optional preview pane supported by prototype.
- Rows: left icon/status, primary label, secondary subline, right metadata/keybind/status.
- Active row: amber left rail, soft amber background, selected icon glow.
- Footer: optional key hints (`up/down move`, `enter run`, `esc close`) and result count.
- Empty state: centered "no command matches" plus suggestion text.

Functional:
- Fuzzy subsequence matcher with contiguous and word-boundary scoring.
- Keyboard:
  - `ArrowDown` / `Ctrl+N`: next result.
  - `ArrowUp` / `Ctrl+P`: previous result.
  - `Enter`: execute active result.
  - `Esc`: clear query if non-empty, otherwise close palette.
- Mouse:
  - Hover moves active selection.
  - Click executes row.
  - Backdrop click closes.
- Groups and modes from prototype:
  - Unified search: actions, agents, navigation, sessions.
  - Actions mode: commands, layout, session, app.
  - Agents mode: agent rows.
  - Navigation mode: workspaces and worktrees.
  - Search mode: scrollback search results.

Prototype note:
The exported `CommandPalette` component currently calls `buildResults("actions", query)` even though `CP_MODES` and screenshots show broader typed modes. Treat full-mode support as intended design functionality, but implement in safe slices against real app data.

## Tech Stack
- Rust 2021.
- `unshit` UI framework in `crates/unshit-framework/`.
- App source in `src/`.
- Styles in `assets/styles.css`.
- Tests through Rust unit tests and `unshit-test`.

## Commands
- Format: `cargo fmt --check`
- Focused tests: `cargo test -p terminal-manager command_palette`
- App tests: `cargo test -p terminal-manager`
- Lint: `cargo clippy -p terminal-manager -- -D warnings`
- Manual run: `cargo run`

## Project Structure
- `src/ui/command_palette.rs` -> palette UI tree, filtering, command rows.
- `src/state.rs` -> palette state, active selection, query, execution dispatch.
- `src/keybinds/mod.rs` -> editable command-palette and rename-session bindings.
- `src/keybinds/registry.rs` -> startup shortcut registry.
- `src/main.rs` -> root overlay mount.
- `assets/styles.css` -> visual implementation.
- `specs/command-palette-redesign.md` -> this spec.
- `docs/design/command-palette-handoff/` -> extracted design handoff reference.

## Code Style
Use small command descriptors and dispatch ids instead of hard-coded click logic per row:

```rust
struct PaletteCommand {
    id: &'static str,
    label: &'static str,
    detail: &'static str,
    keywords: &'static [&'static str],
}

const COMMANDS: &[PaletteCommand] = &[PaletteCommand {
    id: "rename_current_terminal",
    label: "Rename current terminal",
    detail: "Active session",
    keywords: &["rename", "terminal", "session", "title"],
}];
```

CSS must use existing design tokens: `--bg-*`, `--fg-*`, `--border-*`, `--amber-300`, spacing tokens, and radius <= 8px unless the downloaded design requires otherwise.

## Testing Strategy
- State tests:
  - `palette.toggle` opens/closes.
  - command query updates.
  - selection advances/wraps with next/previous commands.
  - `Esc` clears query first, closes second.
  - executing rename closes palette and opens `ConfirmDialog::RenameSession`.
- UI tests:
  - closed palette returns hidden element.
  - open palette renders input and grouped rows.
  - active row class moves with selection.
  - input submit executes selected command.
  - row click executes command.
  - no-match state renders empty row.
- Registry tests:
  - `Ctrl+Shift+P` opens palette.
  - rename shortcut remains unique and dispatches `session.rename_active`.

## Boundaries
- Always:
  - Preserve non-blocking PTY write path.
  - Keep rename RPC daemon-backed.
  - Keep overlay outside render hot path except normal rebuilds from input/dispatch.
  - Run focused tests, full app tests, fmt, and clippy.
- Ask first:
  - Adding dependencies.
  - Changing framework input/focus behavior.
  - Adding command execution that mutates sessions beyond rename.
  - Changing the global keybinding model.
- Never:
  - Remove eager PTY spawning.
  - Revert unrelated dirty work.
  - Commit secrets or local auth data.
  - Show prototype fake sessions/agents/workspaces as real rows.

## Success Criteria
- Local source design is saved under `docs/design/command-palette-handoff/`.
- Palette visually matches the approved/downloaded design at desktop size.
- `Ctrl+Shift+P` opens the redesigned palette.
- Fuzzy filtering works for command labels and keywords.
- Arrow keys and `Ctrl+N` / `Ctrl+P` move selected row.
- Enter executes selected row.
- Clicking a command executes it.
- `Esc` clears query first; second `Esc` closes palette.
- `Rename current terminal` opens the rename dialog for the active terminal.
- Real sessions and workspaces appear when corresponding app state exists.
- Unsupported prototype commands do not appear as executable rows until wired to real dispatch behavior.
- Tests and quality gates pass.

## Open Questions
1. Should `Ctrl+K` remain as editable command-palette default, or should `Ctrl+Shift+P` become the displayed/default binding?
2. Do you want the first implementation slice to include full typed modes (`>`, `@`, `:`, `/`) or only the action palette shell plus real command rows?
3. Which prototype action rows should be executable now besides rename?
4. Should scrollback search be included now even though it needs new searchable buffer plumbing?
