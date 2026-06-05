# Implementation Plan: Command Palette Redesign

## Overview
Replace the current minimal command palette with the downloaded handoff design and full typed-mode behavior. The production palette must use real application state only: safe commands are executable, navigation rows come from real workspaces/tabs/panes, agent and scrollback modes show honest empty states unless real data is available.

## Architecture Decisions
- Create a neutral palette model module (`src/command_palette.rs`) for parsing, fuzzy ranking, safe action descriptors, real-state result projection, and execution IDs. This keeps `state.rs` from depending on `src/ui/*`.
- Keep the visual renderer in `src/ui/command_palette.rs`. It should render grouped result views from the model and dispatch stable IDs through `state::dispatch`.
- Store only volatile interaction state in `AppState`: open flag, query, active flattened index. Mode is derived from the query prefix.
- Route safe commands through existing dispatch arms (`session.rename_active`, `pane.split_right`, `pane.split_down`, `tab.new`, `pane.close`, `sidebar.toggle`, `modal.open`).
- Do not introduce fake agents, fake worktrees, or fake scrollback rows. Empty states are explicit product behavior for unavailable data.
- Change editable command-palette default to `Ctrl+Shift+P`. Keep `Ctrl+K` only as a non-conflicting compatibility alias.

## Dependency Graph
```text
KeybindAction default + shortcut registry
    -> startup shortcut registration
    -> settings effective keybind display

Palette model (modes, fuzzy, result groups, item ids)
    -> AppState palette selection/execution dispatch
    -> UI grouped list, preview, footer, empty states

AppState real workspace/tab/pane snapshot
    -> navigation/session rows
    -> terminal.focus/workspace.switch execution

Framework keyboard event behavior
    -> ArrowUp/ArrowDown while palette input focused
    -> palette select prev/next dispatch

CSS handoff shell
    -> UI element classes
    -> visual verification
```

Implementation order follows the graph: model first, then state execution, then keyboard, then UI/rendering, then visual polish.

## Task List

### Phase 1: Palette Model And Editable Default

## Task 1: Add Palette Result Model

**Description:** Create the shared palette model and pure helpers for mode parsing, fuzzy scoring, grouped result construction, and safe action descriptors. Initial real data includes safe actions, workspaces, tabs, panes, and sessions from `UiSnapshot`; agents and scrollback return mode-aware empty groups when unavailable.

**Acceptance criteria:**
- [ ] `parse_palette_query` maps no prefix, `>`, `@`, `:`, and `/` to the correct mode and stripped query.
- [ ] Fuzzy matcher supports subsequence matching with contiguous and word-boundary score boosts.
- [ ] Safe actions list includes exactly rename, split right, split down, new terminal, close pane, toggle sidebar, and open settings.
- [ ] Result builder emits real navigation/session rows from `UiSnapshot` and no prototype fake rows.

**Verification:**
- [ ] Focused tests pass: `cargo test -p terminal-manager command_palette`
- [ ] Compile check through focused test command.

**Dependencies:** None

**Files likely touched:**
- `src/command_palette.rs`
- `src/main.rs`

**Estimated scope:** Small

## Task 2: Make Ctrl+Shift+P The Editable Default

**Description:** Change the command palette default keybind from `Ctrl+K` to `Ctrl+Shift+P`, keep settings editability through existing keybind state, and preserve `Ctrl+K` as an alias only if it does not create duplicate registered combos.

**Acceptance criteria:**
- [ ] `KeybindAction::CommandPalette.default_combo_str()` returns `Ctrl+Shift+P`.
- [ ] Settings keybind state resolves command palette to `Ctrl+Shift+P` when no override exists.
- [ ] Shortcut registry registers `Ctrl+Shift+P -> palette.toggle`.
- [ ] `Ctrl+K` remains registered as `palette.toggle` compatibility alias and does not duplicate the editable default.

**Verification:**
- [ ] Focused tests pass: `cargo test -p terminal-manager keybind`

**Dependencies:** None

**Files likely touched:**
- `src/keybinds/mod.rs`
- `src/keybinds/registry.rs`
- `src/keybinds/state.rs`

**Estimated scope:** Small

### Checkpoint: Foundation
- [ ] `cargo test -p terminal-manager command_palette`
- [ ] `cargo test -p terminal-manager keybind`
- [ ] No app implementation rows use prototype data.

### Phase 2: Executable Palette Behavior

## Task 3: Add Palette State Selection And Safe Execution

**Description:** Extend palette state handling with active index selection, query reset rules, escape behavior, and execution of all safe action IDs plus real navigation IDs. Keep execution centralized in `state::dispatch`.

**Acceptance criteria:**
- [ ] Opening palette clears query and active selection.
- [ ] Closing palette clears query and active selection.
- [ ] Query updates reset active selection to zero.
- [ ] `palette.select_next` and `palette.select_prev` wrap across flattened current results.
- [ ] `palette.escape` clears query first and closes on second escape.
- [ ] `palette.execute_active` executes the active result when present.
- [ ] `palette.execute:<id>` dispatches the correct safe command or real navigation command, closes palette only on handled execution, and refuses unknown/destructive IDs.

**Verification:**
- [ ] Focused tests pass: `cargo test -p terminal-manager palette`
- [ ] Existing state dispatch tests still pass: `cargo test -p terminal-manager state::tests::dispatch_palette`

**Dependencies:** Task 1

**Files likely touched:**
- `src/state.rs`
- `src/command_palette.rs`

**Estimated scope:** Medium

## Task 4: Wire Keyboard Navigation Without Breaking Terminal Input

**Description:** Route palette-only keyboard controls (`ArrowDown`, `ArrowUp`, `Ctrl+N`, `Ctrl+P`, `Enter`, `Escape`) into palette dispatch while the palette is open. First try app-level raw key/system shortcut handling; if arrow keys are blocked by focused input handling, make the smallest framework-level change required and cover it with regression tests.

**Acceptance criteria:**
- [ ] `Ctrl+Shift+P` opens the palette.
- [ ] `ArrowDown` and `Ctrl+N` move to the next result while palette is open.
- [ ] `ArrowUp` and `Ctrl+P` move to the previous result while palette is open.
- [ ] `Enter` executes the active result while palette is open.
- [ ] `Escape` clears query first, then closes.
- [ ] The same keys do not trigger palette behavior when palette is closed.
- [ ] Terminal keyboard capture remains unchanged outside palette mode.

**Verification:**
- [ ] Focused tests pass: `cargo test -p terminal-manager palette`
- [ ] Framework key-routing regression test passes if framework code is touched: `cargo test -p unshit-app keyboard`
- [ ] App keybind tests pass: `cargo test -p terminal-manager keybind`

**Dependencies:** Tasks 2 and 3

**Files likely touched:**
- `src/main.rs`
- `src/state.rs`
- `src/keybinds/registry.rs`
- `crates/unshit-framework/crates/unshit-app/src/app.rs` only if blocked

**Estimated scope:** Medium

### Checkpoint: Behavior
- [ ] Safe commands execute through existing dispatch.
- [ ] Palette keyboard behavior works in tests.
- [ ] No synchronous PTY IPC added.

### Phase 3: Production UI And Real Modes

## Task 5: Render Handoff-Style Grouped Palette UI

**Description:** Replace the current single-action overlay with a grouped, typed-mode palette shell based on the handoff structure: scrim, panel, input row, mode chip, mode pills, grouped list, active row, footer, and preview pane.

**Acceptance criteria:**
- [ ] Closed palette returns hidden element.
- [ ] Open palette renders `cp-scrim`/overlay, `cp` panel, input, body, grouped list, footer, and preview.
- [ ] Active typed mode shows the correct chip for `>`, `@`, `:`, and `/`.
- [ ] Empty query shows mode hint pills.
- [ ] Active row class follows `snap.palette_active`.
- [ ] Row click dispatches `palette.execute:<id>`.
- [ ] Input submit dispatches `palette.execute_active` after syncing query.

**Verification:**
- [ ] Focused UI tests pass: `cargo test -p terminal-manager command_palette`

**Dependencies:** Tasks 1 and 3

**Files likely touched:**
- `src/ui/command_palette.rs`
- `src/command_palette.rs`
- `src/state.rs`

**Estimated scope:** Medium

## Task 6: Implement Real Mode Content And Honest Empty States

**Description:** Complete typed modes against real data: actions mode groups safe commands, unified mode mixes safe actions with real navigation/session rows, navigation mode can switch/focus real workspaces and terminals, agents and scrollback modes render honest empty states when no real source exists.

**Acceptance criteria:**
- [ ] `>` actions mode shows the seven safe executable commands grouped by command/layout/session/app.
- [ ] Unified mode includes safe commands plus real workspaces, tabs, panes, and sessions where present.
- [ ] `:` navigation mode shows real workspaces/tabs/panes and executing rows uses `workspace.switch` or `terminal.focus`.
- [ ] `@` mode renders no fake agents and shows an honest empty state if no real agent rows exist.
- [ ] `/` mode renders no fake scrollback and shows an honest empty state if no read-only scrollback source exists.
- [ ] No unsupported prototype actions appear as executable rows.

**Verification:**
- [ ] Focused tests pass: `cargo test -p terminal-manager command_palette`
- [ ] State navigation execution tests pass: `cargo test -p terminal-manager terminal_focus`

**Dependencies:** Tasks 1, 3, and 5

**Files likely touched:**
- `src/command_palette.rs`
- `src/ui/command_palette.rs`
- `src/state.rs`

**Estimated scope:** Medium

## Task 7: Apply Handoff CSS And Visual Regression Coverage

**Description:** Replace the old `.command-palette-*` styling with handoff-equivalent classes and add layout/style tests where the local harness can verify panel width, row active rail, footer, preview, and responsive constraints.

**Acceptance criteria:**
- [ ] Scrim is fixed, dark, blurred, top-centered around `12vh`.
- [ ] Panel target width is `760px`, viewport constrained, max height bounded, radius 8px or less.
- [ ] Active row has amber left rail and soft amber background.
- [ ] Mode chips/pills, footer key hints, empty state, and preview pane have dedicated CSS.
- [ ] Mobile/narrow viewport constraints prevent overflow.
- [ ] Visual output is checked with `cargo run` when practical.

**Verification:**
- [ ] Style/UI tests pass: `cargo test -p terminal-manager command_palette`
- [ ] Format passes: `cargo fmt --check`
- [ ] App tests pass: `cargo test -p terminal-manager`
- [ ] Lint passes: `cargo clippy -p terminal-manager -- -D warnings`
- [ ] Manual run: `cargo run`

**Dependencies:** Tasks 5 and 6

**Files likely touched:**
- `assets/styles.css`
- `src/ui/command_palette.rs`
- `src/main.rs` tests only if harness coverage is added there

**Estimated scope:** Medium

### Checkpoint: Complete
- [ ] All success criteria in `SPEC.md` met or explicitly documented as unavailable real data.
- [ ] No fake prototype rows in production output.
- [ ] Focused tests, full tests, fmt, clippy pass.
- [ ] Manual palette check with `cargo run`: `rename`, `>split`, `@`, `:`, `/`.

## Risks and Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| Arrow keys are consumed by focused text input before app shortcuts. | Medium | Test early in Task 4; make minimal framework change only if blocked. |
| State and UI duplicate result-building logic. | Medium | Put result model in `src/command_palette.rs` and import from both state and UI. |
| Navigation rows execute against stale saved workspace data. | Medium | Build IDs from `UiSnapshot` and route through existing tested `workspace.switch`/`terminal.focus` arms. |
| Closing last pane through palette leaves empty workspace unexpectedly. | Low | This matches existing `pane.close` behavior; tests should assert dispatch only, not redefine pane lifecycle. |
| Scrollback mode cannot search real terminal output from current snapshot. | Low | Render honest empty state; do not add daemon IPC in this change. |
| Framework change could affect input handling globally. | Medium | Keep change palette-gated or raw-hook-specific; add framework regression tests. |

## Parallelization Opportunities
- Tasks 1 and 2 can run in parallel.
- Task 7 CSS work can start after Task 5 renders stable classes.
- Tasks 3 through 6 are sequential because they share the result contract and state dispatch behavior.

## Open Questions
- None blocking. User approved full scope, safe command set, `Ctrl+Shift+P` default with settings editability, and boundaries.
