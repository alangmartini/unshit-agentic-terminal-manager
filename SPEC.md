# Spec: Built-in File Editor (MVP)

## Objective
Add a built-in file editor to the terminal manager so users can open, edit, and save
text files in an editor pane that lives inside the existing tab/split layout, next to
terminal panes.

Performance target is "Zed-like": typing latency indistinguishable from the terminal
pane, smooth scrolling through large files, no per-keystroke full-tree rebuild jank.
Usability target is "VS Code-handy" for the core loop: familiar keybindings, selection
and clipboard behavior, undo/redo, dirty indicator, line numbers, mouse support.

This MVP delivers the core editing loop. It explicitly defers syntax highlighting,
in-file search, soft wrap, multiple cursors, IME preedit, a file tree, and LSP —
but the architecture (per-cell fg/bg attributes on `CellGrid`, a standalone buffer
module) must leave those straightforward to add later.

Key architectural facts this spec is built on (verified in the codebase):

- `CellGrid` (`crates/unshit-framework/crates/unshit-core/src/cell_grid.rs`) is the
  terminal's render primitive and is explicitly documented for code editors. It gives
  GPU-fast, damage-tracked monospace rendering with per-cell colors/attributes, stable
  per-line `line_id`s for the renderer's `LineQuadCache`, overscan rows, and
  `set_render_offset_y` for smooth scroll. The editor reuses it verbatim.
- The pane-render seam already branches on "does a `CellGrid` exist for this pane id"
  (`src/ui/terminal_grid.rs:682`), so an editor pane can publish its own grid into the
  same per-frame `grids` map and inherit split/tab/focus machinery.
- `Pane` (`src/state.rs:748`) has no content-type field; pane ≡ terminal is assumed via
  the `AppState.terminals: HashMap<u32, SharedTerminal>` side map. The editor follows
  the same precedent with an `editors` side map, and terminal-assuming call sites gain
  guards.
- The framework's editable element (`InputState`) is single-line only. The multi-line
  edit model is app-owned, driven by a `KeyboardCapture` handler exactly like the
  terminal pane (`captures_keyboard(true)` + handler), reusing the selection/word
  -boundary design from `crates/unshit-framework/crates/unshit-core/src/input.rs`.

## Tech Stack
- Rust 2021, `terminal-manager` workspace package.
- Local `unshit` framework in `crates/unshit-framework/` (`CellGrid`, keyboard capture,
  clipboard via existing paths).
- `rfd` (already a dependency) for the native open-file dialog.
- **No new dependencies.** The buffer is hand-rolled (`Vec<String>` line storage with
  virtualized rendering); no ropey/syntect/tree-sitter in this MVP. House style prefers
  hand-rolled primitives, and `CellGrid` virtualization means render cost is bounded by
  viewport size, not file size.

## Commands
Smallest useful first, then broaden:

- Format: `cargo fmt --check`
- Focused buffer tests: `cargo test -p terminal-manager editor_buffer`
- Focused editor state/dispatch tests: `cargo test -p terminal-manager editor`
- Keybind parity tests: `cargo test -p terminal-manager keybind`
- Full app tests: `cargo test -p terminal-manager`
- Lint: `cargo clippy -p terminal-manager -- -D warnings`
- Manual run: launch via the isolation profile (`TM_PROFILE` / `scripts/tm-isolation.ps1`),
  never against the installed app's daemon.

## Project Structure
- `src/editor/mod.rs`
  - `EditorId`, `EditorPane` (path, buffer, viewport top line, horizontal offset,
    dirty flag, correlation id), open/save entry points.
- `src/editor/buffer.rs`
  - `EditorBuffer`: `Vec<String>` lines, cursor `(line, byte_col)`, selection anchor,
    grouped undo/redo stacks. Pure, heavily unit-tested. All editing ops
    (insert char/str, delete, newline, word ops, selection ops, clipboard slices) live
    here as pure functions on the buffer — no UI or state imports.
- `src/editor/telemetry.rs`
  - Structured JSONL sink modeled on `src/renderer_telemetry.rs` (bounded, rotating,
    correlation ids, **never any file content — paths only**).
- `src/editor/grid.rs`
  - `publish_editor_grid(&EditorPane, rows, cols) -> CellGrid`: visible window +
    overscan into a `CellGrid` with stable `line_id`s, gutter with line numbers,
    cursor via `set_cursor`, selection via bg color.
- `src/state.rs`
  - `pub editors: HashMap<u32, EditorPane>` side map (mirrors `terminals`).
  - Dispatch arms: `editor.open` (rfd dialog), `editor.open:<path>`, `editor.save`.
  - Guards at every terminal-assuming call site for the active pane (spawn/close/
    resize/copy/rename paths, e.g. `state.rs:5482, 5872, 6014, 7115`): if the pane id
    is in `editors`, take the editor path or no-op safely.
- `src/ui/editor_pane.rs`
  - Pane body for editor panes: grid element with `.with_grid(...)`,
    `.with_persistent_buffer(true)`, `.captures_keyboard(true)`, `.with_tab_index(0)`,
    `KeyboardCapture` handler translating keys into `EditorBuffer` ops; mouse click →
    cursor placement, drag → selection, wheel → scroll.
- `src/ui/terminal_grid.rs` / `src/main.rs`
  - Per-frame grid publication (`main.rs:1323–1360`) also publishes editor grids;
    the resize-poll loop (`bridge.rs`, `main.rs:1214`) must not attempt PTY resize for
    editor panes.
- `src/keybinds/mod.rs` + `src/keybinds/registry.rs`
  - New `KeybindAction::{OpenFile, EditorSave}` → `"editor.open"`, `"editor.save"`.
    Keep the registry↔dispatch parity test green (`keybinds/mod.rs:373`).
- `src/command_palette.rs`
  - Palette actions "Open file…" → `editor.open`, "Save file" → `editor.save`
    (added to the safe-dispatch allowlist, `state.rs:4524`).
- `changelog.d/unreleased/`
  - Fragment for the feature commit(s).

## Code Style
- The buffer is pure and standalone: `EditorBuffer` methods take/return plain data,
  no `AppState`, no framework types. Mirrors the style of framework `input.rs`
  (`apply_key_with_mods`) but multi-line.
- Cursor columns are **byte offsets clamped to char boundaries** (same convention as
  `InputState.cursor_pos`); vertical movement preserves a sticky visual column.
- Editing operations return a minimal damage description (dirty line range) so grid
  publication can keep stable `line_id`s for unchanged lines.
- Undo units group consecutive same-kind edits (typing run, single delete run) and
  break on cursor jumps, save, or kind change — VS Code-like granularity.
- No fake data, no placeholder UI. Dirty indicator and titles reflect real state.

## Functional Requirements

Opening:
- `editor.open` opens the native `rfd` file dialog; on selection, opens the file in a
  **new editor pane in a new tab** (tab title = file name).
- `editor.open:<path>` opens a specific path (palette/tests/future file-tree hook).
- Files are read as UTF-8; invalid UTF-8 is refused with a notification (lossy
  conversion is NOT applied silently). Line endings detected (`\n` vs `\r\n`),
  preserved on save, stored internally as `\n`.
- Files larger than 16 MiB are refused with a notification in this MVP (telemetry
  event records the size); the limit is a named constant.

Editing (all via `KeyboardCapture` on the focused editor pane):
- Printable chars insert at cursor (replacing selection if any). Enter inserts a
  newline. Tab inserts 4 spaces (constant). Backspace/Delete work on char, on
  selection, and with Ctrl for word granularity.
- Movement: arrows; Home/End (line), Ctrl+Home/End (document), Ctrl+Left/Right
  (word, using the same word-class rules as framework `input.rs`), PageUp/PageDown
  (viewport height). All movement + Shift extends selection; movement without Shift
  clears it.
- Ctrl+A selects all. Ctrl+C copies selection, Ctrl+X cuts, Ctrl+V pastes (multi-line
  paste supported) — using the same clipboard path the terminal copy uses.
- Ctrl+Z undo, Ctrl+Y and Ctrl+Shift+Z redo.
- Ctrl+S saves: write atomically (temp file + rename) with original line endings;
  clears dirty flag; failure surfaces a notification and telemetry event, keeps dirty.
- Global app keybinds (splits, tab switching, palette) keep working: the capture
  handler must not swallow combos that resolve to registered app shortcuts.

Mouse:
- Click places cursor (gutter-aware: clicks in the gutter select the line).
- Drag selects. Double-click selects word. Wheel scrolls vertically;
  Shift+wheel scrolls horizontally.

Rendering:
- Gutter: right-aligned line numbers, dim fg, width = digits of last line + padding.
- No soft wrap in MVP: long lines scroll horizontally (cursor keeps itself in view).
- Only the viewport window (+overscan) is published to the `CellGrid`; `line_id`s are
  stable across scrolls so `LineQuadCache` replays unchanged lines.
- Cursor uses `set_cursor`/`set_cursor_visible`; selection is rendered as bg color on
  the selected cells; the pane title area shows `<filename>` with a `●` dirty marker.

Lifecycle:
- Closing an editor pane with unsaved changes prompts via the existing dialog
  mechanism (save / discard / cancel). Closing a clean editor pane just closes.
- Editor panes participate in splits, tab switching, and focus exactly like terminal
  panes. Pane resize re-derives rows/cols from cell metrics and re-publishes the grid;
  it must never enter the PTY resize path.
- Editor panes are **not persisted** across restarts in this MVP (unsaved-buffer
  restore is out of scope); a session restart drops editor panes cleanly.

Observability (part of the feature, not a follow-up):
- JSONL sink `editor-events.jsonl` (bounded + rotating, per `renderer_telemetry.rs`
  pattern) with `event`, `level`, `correlation_id` (one id per editor pane instance),
  and machine-readable fields. Events at minimum: `editor.open`, `editor.open_failed`
  (reason: too_large / invalid_utf8 / io), `editor.save`, `editor.save_failed`,
  `editor.close_dirty_prompt`. **Never file content in telemetry; paths and sizes only.**
- Diagnostics events recorded through the existing `DiagnosticEventStore` for
  open/save/close transitions so the diagnostics server can replay them.
- Telemetry writes stay off the keystroke hot path (open/save/close only).

## Testing Strategy
Buffer unit tests (`src/editor/buffer.rs`, pure — the bulk of coverage):
- Insert/delete/newline at start/middle/end of line and document; empty file; empty
  last line semantics; multi-byte UTF-8 chars (é, emoji) never split.
- Word movement/deletion parity with framework word rules.
- Selection: shift-extend in all directions, replace-on-type, delete-selection,
  select-all, clipboard slice extraction for multi-line selections.
- Undo/redo: grouping of typing runs, undo restores cursor+selection, redo cleared on
  new edit, save does not clear undo history.
- Sticky column across short lines; PageUp/Down clamping; CRLF detect/preserve
  round-trip.

State/dispatch tests (inline in `state.rs`, seeding `editors` directly):
- `editor.open:<path>` (temp file) creates a new tab with an editor pane, `editors`
  map populated, title set.
- `editor.save` writes to disk, clears dirty; save failure (readonly path) keeps dirty.
- Refusal paths: oversized file, invalid UTF-8 → no pane created, notification set.
- Active-pane guards: terminal-only dispatches (`session.rename_active`, copy, etc.)
  no-op safely when the active pane is an editor; `pane.close` on a dirty editor opens
  the confirm dialog; on a clean editor closes it.
- Keybind registry↔dispatch parity stays green with the new actions.

UI tests (like `terminal_grid.rs` grid tests / `unshit-test`):
- Editor pane body renders a grid with gutter + content, captures keyboard when
  active, does not when inactive.
- Synthetic keystrokes through the capture handler mutate the buffer (typing "abc"
  shows in grid cells at expected positions; Ctrl+S invokes save).
- Grid publication: viewport window only, stable `line_id`s across a scroll step.

Telemetry test:
- Trigger open + save on a temp file; assert the JSONL sink contains the events with
  expected fields and no file content (mirror `renderer_telemetry.rs:93` privacy test).

Manual check (isolation profile):
- Open a large-ish real file (e.g. `src/state.rs`), scroll fast, type, verify no lag.
- Edit, save, verify on disk; verify dirty marker; split editor next to a terminal;
  confirm terminal keybinds unaffected.

## Boundaries
Always:
- Preserve non-blocking PTY write path; `DaemonPty::write()` stays fire-and-forget.
- Keep rebuild coalescing; prefer redraws over rebuilds. Editor keystrokes must not
  trigger more work per frame than terminal output does.
- Keep the buffer module pure and dependency-free.
- Atomic saves (temp + rename). Never truncate a file on a failed write.
- Emit the telemetry events listed above and verify they appear.
- Use the instance-isolation profile for manual launches.
- Changelog fragment per feat/fix commit.

Ask first:
- Adding any dependency (including ropey/syntect/tree-sitter).
- Changing framework input/focus behavior or `InputState`.
- Persisting unsaved buffers / editor sessions.
- Adding editor state to the daemon or any new IPC.
- Growing `Pane` with new fields instead of using the side map.

Never:
- Load whole files into the render tree (viewport publication only).
- Put file content, or anything derived from it, in telemetry.
- Silently drop or lossy-convert file bytes (refuse instead).
- Add synchronous disk I/O to the render/keystroke path (saves happen on dispatch,
  reads on open; both outside the per-frame build).
- Remove eager PTY spawning in `main.rs`; break terminal panes' behavior.

## Success Criteria
- `editor.open` (palette + keybind) opens a real file into an editor pane in a new
  tab; typing/selection/clipboard/undo/redo/save all work as specified.
- Typing and scrolling in a ~15k-line file feel as responsive as the terminal pane
  (no full-file publication; stable line ids verified by test).
- Dirty tracking + close-confirm work; saves are atomic and preserve line endings.
- Terminal panes are unaffected (guards verified by tests); registry↔dispatch parity
  test passes.
- Telemetry events exist and are queryable in `editor-events.jsonl`.
- `cargo fmt --check`, `cargo clippy -p terminal-manager -- -D warnings`, and
  `cargo test -p terminal-manager` all pass.

## Open Questions
- None blocking for the MVP as scoped. Deferred items (syntax highlighting, in-file
  search, soft wrap, multi-cursor, IME, file tree, persistence) are listed in the
  Objective and intentionally out of scope.
