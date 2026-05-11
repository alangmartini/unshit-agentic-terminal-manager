# Implementation Plan: Desktop Regression Diagnostic Producer Fields

## Overview
Wire producer-side diagnostic snapshot fields in a narrow vertical slice:
schema/test contract first, app-state producers second, documentation and
release artifact last. The work avoids pixel/composition/basic-snapshot changes
owned by other closure workers.

## Dependency Graph
`FrameMetrics` and PTY input/output observations
    -> app diagnostic state in `AppState`
    -> `snapshot.rs` collection
    -> shared schema serialization
    -> docs and changelog artifact

Terminal cursor/scrollback/buffer state
    -> active terminal handle lookup
    -> `TerminalSnapshot` fields

PTY session mappings
    -> `DaemonPty::session_id` / `sessions_iter`
    -> `PtySnapshot` and `active_session_id`

## Architecture Decisions
- Use `DaemonPty` session mappings for active/session liveness because the UI
  sessions panel cache is only refreshed on demand.
- Store renderer frame counter and last-present epoch milliseconds in
  `AppState` from the existing `on_frame_metrics` callback.
- Store PTY recent events as content-free strings that include event kind and
  byte counts, not terminal payload.
- Add only an optional `terminal.buffer_window` schema field, gated by
  `SnapshotOptions.include_terminal_buffer`.
- Leave `renderer.glyph_atlas` unset because the current app callback exposes
  glyph counts and fill ratio, not atlas page counts.

## Task List

### Phase 1: Snapshot Contract
- [x] Task 1: Add failing tests for live diagnostic producer fields.
  - Acceptance: synthetic live state asserts non-null terminal, PTY, and
    renderer fields; default buffer contents remain absent.
  - Verification: `cargo test --bin terminal-manager diagnostics`
  - Dependencies: None
  - Files likely touched: `src/diagnostics/snapshot.rs`
  - Estimated scope: Small

### Checkpoint: Contract
- [x] Tests fail before implementation for the newly expected fields.

### Phase 2: Producer Wiring
- [x] Task 2: Wire app diagnostic state and snapshot collection.
  - Acceptance: producers fill active session id, PTY sessions/recent events,
    terminal cursor/scrollback, renderer frame counter/last-present time, and
    gated buffer window.
  - Verification: `cargo test --bin terminal-manager diagnostics`
  - Dependencies: Task 1
  - Files likely touched: `src/state.rs`, `src/main.rs`, `src/bridge.rs`,
    `src/ui/terminal_grid.rs`, `src/diagnostics/snapshot.rs`,
    `src/diagnostics/server.rs`
  - Estimated scope: Medium

### Checkpoint: Producer
- [x] App diagnostics tests pass.
- [x] No default snapshot path serializes terminal buffer contents.

### Phase 3: Schema And Docs
- [x] Task 3: Add additive schema coverage and honest docs.
  - Acceptance: shared diagnostics crate round-trips optional buffer window;
    docs mention wired snapshot fields and keep event-family caveats.
  - Verification: `cargo test -p terminal-manager-diagnostics`
  - Dependencies: Task 2
  - Files likely touched: `crates/terminal-manager-diagnostics/src/lib.rs`,
    `crates/terminal-manager-diagnostics/tests/serialization.rs`,
    `tests/windows/desktop-regression/README.md`,
    `docs/desktop-regression-debugging.md`
  - Estimated scope: Medium

### Checkpoint: Complete
- [x] `cargo fmt --check`
- [x] `cargo test --bin terminal-manager diagnostics`
- [x] `cargo test -p terminal-manager-diagnostics`
- [x] `cargo test -p xtask desktop_regression`
- [x] `cargo build --bin terminal-manager`

## Risks And Mitigations
| Risk | Impact | Mitigation |
|---|---|---|
| Terminal contents leak in default artifacts | High | Keep buffer window optional and assert default snapshots omit it. |
| PTY session cache remains empty | Medium | Use `DaemonPty` mappings instead of UI sessions panel cache. |
| Renderer internals are overclaimed | Medium | Wire only frame counter/time and cell damage; document glyph atlas caveat. |
| Parallel worker conflicts | Medium | Avoid shared root spec/task files and keep edits in owned diagnostic scope. |

## Open Questions
- None.
