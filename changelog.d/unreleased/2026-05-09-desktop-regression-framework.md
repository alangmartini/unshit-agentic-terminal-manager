# Desktop Regression Framework

Single aggregate changelog fragment for the 10 desktop regression worktrees:
self-tests, result schema, boundaries/versioning, config profiles, preflight
checks, lifecycle isolation, suite contract validation, wait/retry primitives,
authoring ergonomics, and failure diagnostics.

## Added

- Added a Windows desktop regression framework hardening layer with
  non-invasive self-tests, suite validation tests, and compatibility-wrapper
  coverage.
- Added versioned `desktop-regression.results/v1` artifacts with run, source,
  binary, environment, suite timing, failure classification, screenshot, event,
  and diagnostic metadata.
- Added named desktop regression profiles, profile listing, preflight checks,
  suite/tag filtering, and validation before build or desktop control.
- Added owned-process lifecycle tracking, explicit stale-process reporting and
  cleanup, and an owned-process ledger for local desktop runs.
- Added bounded wait/retry primitives and authoring helpers for app sessions,
  before/after captures, geometry assertions, and visual stripe checks.
- Added failure diagnostics that capture manifests, final screenshots,
  foreground window metadata, active app session state, and matching run logs
  before cleanup.

## Changed

- Updated desktop regression suites and templates to use the shared session,
  capture, assertion, and wait helpers instead of ad hoc lifecycle and timing
  code.
- Updated desktop regression compatibility wrappers to forward the expanded
  runner options while preserving existing wrapper entry points.

## Notes

- Full desktop suite execution remains manual because it launches the app,
  controls the real desktop, sends global input, moves the mouse, and captures
  screenshots.
