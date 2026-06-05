# Spec: Command Palette Redesign

## Objective
Replace the current command palette with the exported Claude Design handoff command palette for terminal-manager users and power users.

The palette should work like a VS Code-style launcher inside the Rust terminal manager:

- `Ctrl+Shift+P` opens the palette by default.
- Users can filter, navigate, and execute safe commands without leaving the keyboard.
- Typed modes are supported:
  - unified mode: search everything available.
  - `>` actions mode: executable app commands.
  - `@` agents mode: real agent/session rows when real data exists.
  - `:` navigation mode: real workspaces, tabs, terminals, and worktrees when real data exists.
  - `/` scrollback mode: real terminal output search when readable app data exists.
- The visual result should match the local design handoff, not the current minimal palette.

Reference source:

- Prior repo spec: `specs/command-palette-redesign.md`
- Local handoff root: `docs/design/command-palette-handoff/command-pallete/project/`
- Primary design file: `docs/design/command-palette-handoff/command-pallete/project/Command Palette.html`
- Palette logic: `docs/design/command-palette-handoff/command-pallete/project/command-palette.jsx`
- Palette styling: `docs/design/command-palette-handoff/command-pallete/project/palette.css`

Success means the production UI has the same structure, density, color behavior, command grouping, typed prefixes, fuzzy ranking, keyboard behavior, and result preview language as the handoff, while only showing production data that comes from real app state.

## Tech Stack
- Rust 2021.
- `terminal-manager` workspace package.
- Local `unshit` UI framework in `crates/unshit-framework/`.
- App source in `src/`.
- Styles in `assets/styles.css`.
- Existing Rust unit tests plus `unshit-test` for UI tree/style assertions.
- No new runtime dependencies for this change.

## Commands
Use the smallest useful command first, then broaden before handoff.

- Format: `cargo fmt --check`
- Focused palette tests: `cargo test -p terminal-manager command_palette`
- Focused keybind tests: `cargo test -p terminal-manager keybind`
- Focused state dispatch tests: `cargo test -p terminal-manager palette`
- Full app tests: `cargo test -p terminal-manager`
- Lint: `cargo clippy -p terminal-manager -- -D warnings`
- Manual run for UI/layout check: `cargo run`

## Project Structure
- `src/ui/command_palette.rs`
  - Production palette UI tree.
  - Palette row descriptors, grouped rendering, fuzzy matching, typed mode parsing, preview rendering.
  - Unit tests for UI tree output and interaction callbacks.
- `src/state.rs`
  - Palette state fields.
  - Dispatch handlers for palette open/close/query/selection/mode/execute.
  - Real-data projection for sessions, workspaces, tabs, terminals, and scrollback where available.
- `src/keybinds/mod.rs`
  - `KeybindAction::CommandPalette` default changes from `Ctrl+K` to `Ctrl+Shift+P`.
  - Settings still exposes the command palette keybind as editable.
- `src/keybinds/registry.rs`
  - Startup shortcut registration.
  - Keep compatibility aliases only when they do not conflict with editable defaults.
- `src/ui/settings.rs`
  - Existing keybind settings should display/edit the new default through current keybind plumbing.
- `src/main.rs`
  - Palette overlay mount remains above the terminal UI.
- `assets/styles.css`
  - Replace current `.command-palette-*` styling with handoff-equivalent shell:
    scrim, palette panel, input row, mode chips, grouped list, active row, empty state, preview pane, footer hints.
- `docs/design/command-palette-handoff/`
  - Read-only local design reference.
- `tasks/`
  - Implementation plan and todo files for the implement workflow.

## Code Style
Use data descriptors and dispatch IDs instead of hard-coded per-row UI logic.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PaletteAction {
    id: &'static str,
    label: &'static str,
    description: &'static str,
    group: PaletteGroup,
    icon: PaletteIcon,
    keybind: Option<KeybindAction>,
    dispatch: &'static str,
    keywords: &'static [&'static str],
    enabled: bool,
}

const SAFE_ACTIONS: &[PaletteAction] = &[
    PaletteAction {
        id: "rename_current_terminal",
        label: "Rename current terminal",
        description: "Rename the focused terminal session.",
        group: PaletteGroup::Actions,
        icon: PaletteIcon::Terminal,
        keybind: Some(KeybindAction::RenameSession),
        dispatch: "session.rename_active",
        keywords: &["rename", "terminal", "session", "title"],
        enabled: true,
    },
];
```

Use small pure helpers for behavior that needs tests:

- `parse_palette_query(input) -> (PaletteMode, query_without_prefix)`
- `fuzzy_match(query, text) -> Option<FuzzyScore>`
- `build_palette_results(snapshot, mode, query) -> Vec<PaletteGroupView>`
- `execute_palette_item(state, item_id) -> bool`

Style conventions:

- Match handoff class concepts even if exact class names stay Rust-specific.
- Prefer existing token names in `assets/styles.css`.
- Keep radii at 8px or less.
- Do not introduce fake prototype labels such as `dashboard-dev`, `api.server`, `claude - refactor-userlist`, or fake workspaces.
- Keep text short enough for the 760px target width and mobile-constrained width.

## Functional Requirements
Palette opening:

- Default displayed/editable keybind is `Ctrl+Shift+P`.
- Settings can change the command palette keybind through the existing keybind settings UI.
- Existing `palette.toggle` dispatch remains the command target.
- `Ctrl+K` can remain only as a compatibility alias if it does not interfere with the editable default.

Mode parsing:

- No prefix means unified mode.
- Leading `>` enters actions mode and removes the prefix from the search query.
- Leading `@` enters agents mode and removes the prefix from the search query.
- Leading `:` enters navigation mode and removes the prefix from the search query.
- Leading `/` enters scrollback mode and removes the prefix from the search query.
- Mode chip appears when a typed mode is active.
- Empty query shows mode hints/pills matching the handoff.

Keyboard and mouse:

- `ArrowDown` and `Ctrl+N` move selection to next result with wrap.
- `ArrowUp` and `Ctrl+P` move selection to previous result with wrap.
- `Enter` executes the active result.
- `Esc` clears query first when non-empty. A second `Esc` closes the palette.
- Mouse hover moves active selection.
- Mouse click executes that row.
- Backdrop click closes the palette.

Fuzzy filtering:

- Use subsequence matching.
- Contiguous matches rank above gapped matches.
- Word-boundary matches get a ranking boost.
- Labels, descriptions, IDs, keybind labels, and keywords are searchable.
- Highlight matched characters where the UI framework can represent it cleanly.

Safe real actions:

- `Rename current terminal` -> `session.rename_active`.
- `Split pane right` -> `pane.split_right`.
- `Split pane down` -> `pane.split_down`.
- `New terminal` -> `tab.new`.
- `Close pane` -> `pane.close`.
- `Toggle sidebar` -> `sidebar.toggle`.
- `Open settings` -> `modal.open`.

Do not wire these as executable actions in this change unless a real safe dispatch path already exists and the implementation explicitly tests it:

- kill session
- restart session
- clear scrollback
- change theme
- spawn agent
- new worktree
- grid/balance/fullscreen if current dispatch is absent or unsafe

Data modes:

- Unified mode includes safe actions plus real sessions/navigation/agents that can be projected from `UiSnapshot` or `AppState`.
- Actions mode includes all safe executable actions grouped as commands, layout, session, and app where applicable.
- Agents mode shows real agent-attached terminals or real quick-prompt/agent metadata only if that data exists in app state. If no real agent model exists yet, show an honest empty state for `@`, not fake prototype agents.
- Navigation mode shows real workspaces, tabs, panes, terminals, and worktrees that exist in app state. Executing a navigation row should use existing safe dispatch such as `workspace.switch:<idx>` or `terminal.focus:<ws_idx>:<pane_id>`.
- Scrollback mode searches real terminal output available to the UI. If no read-only scrollback source is available, `/` mode still exists but renders an honest empty state that says no searchable scrollback is available.

Preview pane:

- Match handoff structure where practical: header, title/kicker, detail rows, preview body, and run hint.
- Action previews show description, shortcut, scope, and reversible status.
- Session/navigation/search previews use only real state.
- Preview can be disabled if the current layout or renderer cannot support it cleanly in the first production pass, but the spec target is to include it.

Visual requirements:

- Scrim covers the app with dark translucent background and blur.
- Palette is top-centered around `12vh`, target width `760px`, max width constrained to viewport.
- Panel uses gradient elevated surface, subtle amber glow, border, 6px to 8px radius.
- Input row includes amber prompt, optional mode chip, editable query, and `esc` chip.
- Body has scrollable grouped list.
- Rows use icon/status, primary label, optional subline, right metadata/keybind/status.
- Active row uses amber left rail, soft amber background, and icon glow.
- Empty state is centered and mode-aware.
- Footer hints are supported: move, run, close, result count.

## Testing Strategy
State tests:

- Opening palette clears old query, mode, and active selection.
- Closing palette clears query and selection.
- Query prefix parsing maps `>`, `@`, `:`, `/` to the correct mode.
- Query updates reset active selection.
- Selection next/previous wraps across flattened grouped results.
- `Esc` clears query first and closes on second press.
- Executing each safe action closes palette and dispatches the expected real command.
- Navigation rows dispatch only real workspace/pane focus commands.
- Unsupported/destructive prototype actions are absent or disabled and do not dispatch.
- `Ctrl+Shift+P` is `KeybindAction::CommandPalette` default.
- Command palette keybind remains editable through existing keybind state.

UI tests:

- Closed palette returns hidden element.
- Open palette renders scrim, card, input, list, mode hints, and footer/preview according to state.
- Actions mode renders the seven safe commands.
- Unified mode includes safe commands and real data groups where present.
- `@`, `:`, and `/` modes render their mode chip and honest empty state when no real rows exist.
- Active row class moves with selection.
- Row click executes that row.
- No-match state renders mode-aware empty copy.
- CSS contains handoff-equivalent selectors/properties for scrim, panel width, active rail, mode chip colors, compact rows, preview, and footer.

Manual check:

- Run `cargo run`.
- Open palette with `Ctrl+Shift+P`.
- Try `rename`, `>split`, `@`, `:`, and `/`.
- Confirm rename opens the existing rename dialog for the active pane.
- Confirm split/new/close/sidebar/settings actions match existing toolbar/keybind behavior.

## Boundaries
Always:

- Preserve non-blocking PTY write path.
- Keep `DaemonPty::write()` fire-and-forget.
- Keep cursor blink renderer-side.
- Prefer redraws over rebuilds for renderer-only changes.
- Preserve rebuild coalescing behavior in the framework.
- Use real app data only.
- Add regression tests for behavior touched by the palette.
- Run focused tests before broad tests.
- Keep the design handoff as read-only reference.

Ask first:

- Adding dependencies.
- Changing framework input/focus behavior.
- Changing the keybind persistence file format.
- Adding destructive command execution.
- Adding new daemon IPC for scrollback or agent discovery.
- Changing the PTY lifecycle or eager PTY spawn behavior.
- Making framework-level changes unless the app implementation is blocked.

Never:

- Show fake prototype sessions, agents, workspaces, worktrees, commands, or scrollback as production rows.
- Remove eager PTY spawning in `main.rs`.
- Add synchronous IPC to render path.
- Revert unrelated dirty work.
- Edit released changelog history.
- Commit secrets or local auth data.

## Success Criteria
- `SPEC.md` exists and is standalone.
- `Ctrl+Shift+P` is the default command palette keybind displayed in settings.
- Settings can change the command palette keybind through existing editable keybind flow.
- Palette opens with the redesigned handoff-equivalent UI.
- Full typed modes are present: unified, `>`, `@`, `:`, `/`.
- Fuzzy filtering and ranking works across available rows.
- Keyboard and mouse interactions match requirements.
- Safe commands execute:
  - Rename current terminal.
  - Split pane right.
  - Split pane down.
  - New terminal.
  - Close pane.
  - Toggle sidebar.
  - Open settings.
- Rename command opens the existing active-pane rename dialog.
- Real sessions/navigation/search rows appear only when backed by real state.
- Empty modes are honest and do not use prototype fake data.
- Focused tests, full app tests, formatting, and clippy pass.

## Open Questions
- None blocking. User approved full scope, safe command set, `Ctrl+Shift+P` default with settings editability, and the stated boundaries.
