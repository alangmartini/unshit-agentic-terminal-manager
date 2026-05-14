# Ship Review: Desktop Regression Snap Composition Guard

## Ship Decision: GO

### Blockers

- None.

### Recommended fixes

- None for this closure item.

### Acknowledged risks

- The headed `cargo xtask desktop-regression --suite post-resize-glitches`
  run is manual and was not executed in this pass. Mitigation: automated helper
  tests and `xtask` build/test validation passed; run the headed suite before
  relying on the guard as a desktop-environment signal.
- A concurrent closure item has unstaged changes in
  `post_resize_glitches.rs`. Mitigation: commit only the closure-2 hunks for
  the snap-composition guard.

### Rollback plan

- Trigger conditions: false `snap-foreground-stolen`,
  `snap-stuck-modifier`, or `snap-window-occluded` failures on otherwise clean
  headed desktop runs.
- Rollback procedure: revert the snap-composition guard commit, then rerun
  `cargo test -p xtask desktop_regression` and the manual headed suite.
- Recovery time objective: one local revert and validation cycle.

## Specialist Reports

### code-reviewer

- Correctness: GO. The suite checks snap capture readiness after the existing
  post-snap settle and before post-snap screenshot capture. Failure mapping
  covers the requested first-bad signals.
- Readability: GO. Win32 API calls stay in `win32.rs`; suite code only maps a
  domain error to `SuiteError`.
- Architecture: GO. Non-Windows stubs are preserved and pure occlusion logic is
  test-covered without HWNDs.
- Security: GO. No auth, secrets, persistence, network, or dependency changes.
- Performance: GO. Z-order scan is bounded and only runs once per headed suite
  execution.

### security-auditor

- No Critical or High findings.
- No secrets or sensitive data added.
- Desktop handles and rects are diagnostic metadata only.

### test-engineer

- Coverage added for positive-area overlap, edge-touching non-overlap, hidden
  and owned windows, first visible occluder selection, below-target windows,
  and first-bad signal mapping.
- Remaining gap: real HWND foreground/Z-order behavior requires the manual
  headed Windows suite.
