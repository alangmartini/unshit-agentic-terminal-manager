# Spec: Desktop Regression Diagnostic Producer Fields

## Objective
Wire the producer side of terminal-manager diagnostic snapshots so engineers and
agents debugging Windows desktop regressions can distinguish a healthy terminal
from unwired `null` fields. Success means snapshots collected with diagnostics
enabled expose truthful terminal, PTY, and renderer liveness fields for a live
active terminal, while default snapshots continue to exclude terminal buffer
contents.

## Tech Stack
- Rust application diagnostics under `src/diagnostics/`.
- Shared serde schema in `crates/terminal-manager-diagnostics`.
- Existing terminal state in `src/state.rs`, `src/terminal/mod.rs`, and renderer
  frame callback wiring in `src/main.rs`.

## Commands
- Format: `cargo fmt --check`
- App diagnostics tests: `cargo test --bin terminal-manager diagnostics`
- Shared schema tests: `cargo test -p terminal-manager-diagnostics`
- Desktop regression unit tests: `cargo test -p xtask desktop_regression`
- Build: `cargo build --bin terminal-manager`

## Project Structure
- `src/diagnostics/snapshot.rs`: collect app diagnostic snapshot fields and
  focused snapshot tests.
- `src/diagnostics/server.rs`: pass snapshot options through the authenticated
  command path.
- `src/state.rs`: store minimal diagnostic frame and PTY event state.
- `src/main.rs`, `src/bridge.rs`, `src/ui/terminal_grid.rs`: feed frame and PTY
  observations into app state.
- `crates/terminal-manager-diagnostics/src/lib.rs`: additive schema only when a
  new optional field is needed.
- `tests/windows/desktop-regression/README.md` and
  `docs/desktop-regression-debugging.md`: document only fields/events that are
  actually wired.

## Code Style
Keep producer wiring narrow and explicit:

```rust
let cursor = Some(TerminalCursorSnapshot {
    row: grid.cursor_row().min(u32::MAX as usize) as u32,
    col: grid.cursor_col().min(u32::MAX as usize) as u32,
    visible: grid.cursor_visible(),
});
```

Prefer additive state and schema fields over reinterpreting absent values. When
state is not available, leave the snapshot field absent instead of inventing a
placeholder.

## Testing Strategy
- Add regression tests that seed a live active terminal/session and assert
  cursor, scrollback length, active session id, PTY sessions/recent events,
  renderer frame counter, and last-present timestamp are present.
- Add default-buffer tests proving terminal contents are excluded unless
  `SnapshotOptions.include_terminal_buffer` is true.
- Add schema serialization coverage for optional buffer windows.
- Re-run the existing diagnostics, shared-schema, xtask desktop-regression, and
  build commands before commit.

## Boundaries
- Always: keep diagnostics explicit and off by default; keep terminal buffer
  contents excluded from default snapshots; use truthful producer values.
- Ask first: new dependencies, CI changes, pixel thresholds, or changing
  observe-mode snapshot capture semantics.
- Never: collect terminal buffer contents by default; advertise diagnostic event
  families that are not emitted; fake glyph atlas or renderer internals that the
  current app state does not expose.

## Success Criteria
- A synthetic live app state produces non-null terminal cursor, scrollback
  length, active session id, PTY session list, PTY recent events, renderer frame
  counter, and renderer last-present timestamp.
- An opt-in snapshot includes a bounded terminal buffer window; the default path
  does not include terminal contents and reports
  `config.terminal_buffer_contents_included = false`.
- Diagnostic docs state that terminal/PTY/renderer snapshot fields are wired,
  while event-family caveats remain limited to event streams that are still not
  emitted.
- Validation commands pass.

## Open Questions
- None for this closure item. Glyph atlas pages and GPU-native dirty regions are
  intentionally left unwired until the renderer exposes those exact values.
