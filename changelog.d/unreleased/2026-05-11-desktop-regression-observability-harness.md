# Desktop Regression Observability Harness

## Added

- Added a Rust `cargo xtask desktop-regression` runner for the current Windows
  desktop suites, including headed app launch, Win32 window control,
  screenshots, runner events, results, and failure bundles.
- Added a gated `terminal-manager` diagnostics protocol over Windows named
  pipes with token authentication, snapshots, step markers, invariant
  evaluation, deterministic-mode preparation, event draining, and request
  timeout protection.
- Added record and logical replay support for the `edge-resize-stability`
  desktop suite, with replayed runner input and recorded resize assertions
  against a fresh app process.
- Added interactive failure handling for headed desktop runs, including
  keep-open failure inspection, notes, screenshots, diagnostics capture, and
  abort/continue/close decisions.

## Changed

- Migrated `edge-resize-stability` and `post-resize-glitches` from the legacy
  PowerShell runner path to the Rust desktop regression runner.
- Updated the PowerShell desktop-regression entry points to remain
  compatibility wrappers around the Rust runner.
- Updated desktop regression artifacts to use collision-resistant run ids and
  distinct run start/finish timestamps.

## Fixed

- Fixed replay mode so traces are consumed for supported suites instead of only
  validated and reported in `results.json`.
- Fixed diagnostic capability reporting so the app only advertises event
  families it currently emits: `test_step`, `invariant`, and `log`.
- Fixed the edge-resize restore flow so a failed inward resize reports the
  behavior assertion instead of failing setup on a no-op restore drag.

## Notes

- Full desktop suite execution remains manual because it launches the app,
  controls the real Windows desktop, sends global input, moves the mouse, and
  captures screenshots.
