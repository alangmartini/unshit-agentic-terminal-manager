# Spec: Desktop Regression Basic Step Snapshots

## Objective
Default/basic observed desktop-regression runs should collect step snapshots when
diagnostics are available. Engineers and agents debugging terminal-manager
desktop regressions should get `pre-snap` and `post-snap` diagnostic state by
default so existing cross-layer assertions are not silently skipped outside
`--observe full`.

## Tech Stack
- Rust `xtask` desktop regression runner.
- Shared diagnostic protocol from `terminal-manager-diagnostics`.
- Headed Windows desktop suites remain manual black-box tests.

## Commands
- Format: `cargo fmt --check`
- Focused validation: `cargo test -p xtask desktop_regression`
- Optional headed run: `cargo xtask desktop-regression --suite post-resize-glitches --observe basic`

## Project Structure
- `xtask/src/desktop_regression/suites/observability.rs`: shared diagnostic
  suite helpers and focused helper tests.
- `tests/windows/desktop-regression/README.md`: desktop regression runner docs.
- `docs/desktop-regression-debugging.md`: debugging runbook observe-mode docs.
- `tasks/desktop-regression-basic-step-snapshots-*.md`: closure-specific task
  artifacts for this implementation.

## Code Style
Use small mode predicate helpers when behavior differs by `ObserveMode`:

```rust
fn captures_step_snapshots(observe: ObserveMode) -> bool {
    matches!(observe, ObserveMode::Basic | ObserveMode::Full)
}
```

Keep diagnostics operations explicit in the suite helper and reserve full-only
operations behind `observe == ObserveMode::Full`.

## Testing Strategy
- Add focused unit coverage for the observe-mode predicate/helper behavior.
- Run `cargo test -p xtask desktop_regression` after implementation.
- Do not require a live diagnostic app in unit tests.
- Headed desktop regression runs are manual and not required for automated
  validation in this closure item.

## Boundaries
- Always: preserve real desktop black-box assertions and existing cross-layer
  checks.
- Always: keep deterministic-mode preparation, step markers, invariant
  evaluation, and clear-step behavior full-only.
- Ask first: adding suites, CI, new dependencies, or changing result schema.
- Never: wire producer-side diagnostic fields/events, change pixel thresholds,
  change foreground/occlusion checks, or overwrite shared root `SPEC.md`,
  `tasks/plan.md`, or `tasks/todo.md`.

## Success Criteria
- `capture_step_snapshot` attempts snapshot capture in both
  `ObserveMode::Basic` and `ObserveMode::Full` when diagnostics exist.
- `capture_step_snapshot` still returns `Ok(None)` for `ObserveMode::Off` or
  absent diagnostics.
- Full-only diagnostic operations remain full-only.
- Basic observe-mode documentation states that step snapshots are included, and
  full mode is documented as adding deterministic mode, markers, invariants,
  and extra full-only checks.
- Focused `xtask` tests pass without a live diagnostic app.

## Open Questions
None. The closure brief supplies the clarifying answers and scope boundaries.
