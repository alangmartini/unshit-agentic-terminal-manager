# Plan: Built-in File Editor (MVP)

Spec: `SPEC.md` (file editor). Recon facts baked in: `CellGrid` is the render
primitive (virtualization + line cache solved); `Pane` has no content type — editor
uses an `editors: HashMap<u32, EditorPane>` side map mirroring `terminals`; multi-line
editing is app-owned via `KeyboardCapture` like the terminal pane.

## Dependency graph

```
buffer.rs (pure, no deps)
   ├─> editor/mod.rs (EditorPane: open/save, dirty, viewport)
   │      ├─> state.rs (editors map, dispatch arms, guards)
   │      │      ├─> ui/editor_pane.rs (grid publication + keyboard capture)
   │      │      │      ├─> mouse handling
   │      │      │      └─> main.rs grid publication + resize-guard wiring
   │      │      ├─> keybinds + command palette entries
   │      │      └─> close-dirty dialog
   │      └─> editor/telemetry.rs (sink; events emitted from open/save/close)
   └─> undo/redo + clipboard ops (buffer-internal)
```

Structural risk is concentrated in the state.rs seam (side map + guards at
terminal-assuming call sites) and the main.rs grid-publication/resize loop. Slice 1
retires that risk with the smallest possible feature (read-only viewer).

## Slices (vertical — each lands complete with tests)

### S1 — Read-only file viewer pane (structural seam)
Open a file into a scrollable, read-only editor pane in a new tab.
- `src/editor/mod.rs`: `EditorPane { id, path, buffer, top_line, h_offset, dirty=false,
  correlation_id, line_ending }`; `open_file(path)` with UTF-8/16 MiB refusal.
- `src/editor/buffer.rs`: just the storage type + line access (editing ops come in S2).
- `state.rs`: `editors` map; `editor.open:<path>` dispatch arm → new tab + pane +
  entry in `editors`; notification on refusal.
- `src/editor/grid.rs`: viewport window (+overscan) → `CellGrid` with gutter,
  stable `line_id`s.
- `src/ui/editor_pane.rs`: pane body with grid, `captures_keyboard(true)`,
  handler supporting only scroll keys (arrows/PageUp/Down/Ctrl+Home/End move viewport)
  + wheel scroll.
- `main.rs`: publish editor grids per frame alongside terminal grids; resize-poll loop
  skips editor panes (no PTY resize).
- Guards (minimum viable set): `pane.close`, resize path, copy path no-op safely.
- Telemetry: sink skeleton + `editor.open` / `editor.open_failed`.
- **Acceptance**: dispatch test opens temp file → tab exists, grid renders expected
  cells; oversized/invalid-UTF-8 refused with notification; UI test: grid publishes
  viewport only, stable line_ids across scroll; terminal panes untouched (existing
  tests green).
- **Verify**: `cargo test -p terminal-manager editor`, full `cargo test`, fmt, clippy.

**CHECKPOINT A**: structural seam proven. Manual launch (isolation profile): open
`src/state.rs`, scroll fast; terminals still work. Screenshot for UI verification.

### S2 — Core editing
- `buffer.rs`: insert char/str, newline, backspace/delete (+Ctrl word variants),
  movement (arrows/Home/End/Ctrl+arrows/Ctrl+Home/End/PageUp/Down), sticky column,
  selection (Shift-extend, replace-on-type, Ctrl+A), damage ranges. Multi-byte safe.
- `ui/editor_pane.rs`: full `KeyboardCapture` translation; don't swallow registered
  app shortcuts; cursor via `set_cursor`; selection bg rendering; cursor kept in view
  (auto-scroll vertical + horizontal).
- Dirty flag + `●` title marker.
- **Acceptance**: buffer unit tests (the bulk — edits, movement, selection, UTF-8,
  sticky column); UI test: synthetic "abc" keystrokes appear in grid cells.

### S3 — Clipboard + undo/redo
- Ctrl+C/X/V through the existing clipboard path (multi-line paste); Ctrl+Z/Y/
  Ctrl+Shift+Z with VS Code-like grouping (typing runs; break on jump/kind/save).
- **Acceptance**: buffer tests for grouping, cursor restore, redo-clear-on-edit,
  multi-line clipboard slices; capture-handler tests for the combos.

### S4 — Save + open UX (keybinds, palette, dialog)
- Atomic save (temp + rename), preserve CRLF, clear dirty, failure → notification +
  keep dirty. `editor.save` dispatch.
- `editor.open` arm → `rfd` dialog (off the render path).
- `KeybindAction::{OpenFile, EditorSave}` (defaults: Ctrl+O / Ctrl+S if free in
  registry; capture handler routes Ctrl+S when editor focused), palette actions added
  to safe-dispatch allowlist.
- Telemetry: `editor.save` / `editor.save_failed`.
- **Acceptance**: dispatch tests save/failure/CRLF round-trip; keybind parity test
  green; palette lists the actions.

**CHECKPOINT B**: core loop complete (open→edit→save). Full test suite + fmt +
clippy. Manual: edit+save a real file, verify on disk; split editor next to terminal.

### S5 — Mouse
- Click places cursor (gutter click selects line), drag selects, double-click selects
  word, wheel vertical, Shift+wheel horizontal.
- **Acceptance**: UI tests for click→cursor mapping and drag selection.

### S6 — Lifecycle polish + full guard audit
- Close-dirty confirm dialog (save/discard/cancel) via existing dialog mechanism;
  clean close just closes. Editor panes dropped cleanly on restart (no persistence).
- Audit ALL terminal-assuming call sites for the active pane (state.rs:5482, 5872,
  6014, 7115 et al.) — guard each; dispatch tests prove terminal-only commands no-op
  on editor panes.
- Telemetry: `editor.close_dirty_prompt`; diagnostics events for open/save/close via
  `DiagnosticEventStore`.
- **Acceptance**: dialog state tests; guard tests; privacy test (no file content in
  sink, mirroring `renderer_telemetry.rs:93`).

**CHECKPOINT C (final)**: full gates — fmt, clippy `-D warnings`, full test suite,
manual smoke (isolation profile) + screenshot, telemetry file inspected for expected
events. Changelog fragment written.

## Out of scope (deferred per spec)
Syntax highlighting, in-file search, soft wrap, multi-cursor, IME preedit, file tree,
editor-session persistence, new dependencies.
