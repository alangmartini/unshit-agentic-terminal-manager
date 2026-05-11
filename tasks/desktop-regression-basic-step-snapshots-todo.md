# Todo: Desktop Regression Basic Step Snapshots

- [x] Task 1: Make Basic Eligible For Step Snapshots
  - Acceptance: Basic and full are eligible, off remains ineligible, missing diagnostics still return `Ok(None)`, and full-only behavior remains full-only.
  - Verify: `cargo test -p xtask desktop_regression`; `cargo fmt --check`.
  - Files: `xtask/src/desktop_regression/suites/observability.rs`.
- [x] Task 2: Update Observe-Mode Documentation
  - Acceptance: README and debugging runbook say basic includes step snapshots and full adds deterministic mode, markers, invariants, and extra full-only checks.
  - Verify: `cargo test -p xtask desktop_regression`.
  - Files: `tests/windows/desktop-regression/README.md`, `docs/desktop-regression-debugging.md`.
