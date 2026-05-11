# Ship Review: Desktop Regression Diagnostic Producer Fields

## Ship Decision: GO

### Blockers
- None.

### Recommended Fixes
- None.

### Acknowledged Risks
- `renderer.glyph_atlas` remains unwired because current app state does not
  expose atlas page counts. Mitigation: docs call this out and snapshots leave
  the field absent.
- Headed desktop suites were not run because they control the real Windows
  desktop and send global input. Mitigation: unit, schema, and build validation
  passed; headed execution remains a manual follow-up.

### Rollback Plan
- Trigger conditions: diagnostic snapshots leak terminal contents by default,
  authenticated buffer snapshots expose more than the bounded opt-in window, or
  snapshot collection causes app/runtime instability.
- Rollback procedure: revert the implementation commit, rebuild
  `terminal-manager`, and rerun `cargo test --bin terminal-manager diagnostics`,
  `cargo test -p terminal-manager-diagnostics`, and
  `cargo test -p xtask desktop_regression`.
- Recovery time objective: one revert/build/test cycle.

### Specialist Reports

#### code-reviewer
- Correctness: PASS. Snapshot tests cover live terminal cursor/scrollback,
  active session id, PTY sessions/recent events, renderer frame presence, and
  default-vs-opt-in buffer behavior.
- Readability: PASS. Producer logic is localized in `snapshot.rs`, with narrow
  state helpers for frame and PTY observations.
- Architecture: PASS. Uses existing `DaemonPty`, `Terminal`, `CellGrid`, and
  `on_frame_metrics` surfaces; schema addition is optional/additive.
- Security: PASS. Terminal contents remain excluded by default and require an
  authenticated snapshot request with `include_terminal_buffer`.
- Performance: PASS. New producer state is bounded; PTY events are content-free
  strings with a 32-entry ring, and buffer snapshots are capped.

#### security-auditor
- No secrets, auth, dependency, or configuration changes.
- Data exposure risk is limited to the intentional opt-in buffer window. Default
  snapshots assert `terminal_buffer_contents_included = false` and no
  `buffer_window`.
- No Critical or High findings.

#### test-engineer
- Covered: app diagnostics unit tests, shared schema serialization,
  desktop-regression xtask unit tests, formatting, and app build.
- Gap: no headed desktop regression run in this pass. This is acceptable for
  ship readiness because the requested change is producer-side diagnostic state
  and the headed suites are manual environment checks.

## Validation
- `cargo fmt --check`
- `cargo test --bin terminal-manager diagnostics`
- `cargo test -p terminal-manager-diagnostics`
- `cargo test -p xtask desktop_regression`
- `cargo build --bin terminal-manager`
