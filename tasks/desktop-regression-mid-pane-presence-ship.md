# Ship Decision: Desktop Regression Mid-Pane Presence

## Ship Decision: GO

### Blockers
- None for the mid-pane presence closure.

### Recommended Fixes
- Run the manual headed suite when a desktop baseline is desired:
  `cargo xtask desktop-regression --suite post-resize-glitches`.
- Re-run `cargo fmt --check` after unrelated concurrent formatting diffs are
  resolved.

### Acknowledged Risks
- The lower-bound threshold is intentionally conservative because no healthy
  baseline artifact is on file. Re-baseline on theme, foreground palette,
  antialiasing, sample geometry, or lit-threshold changes.

### Rollback Plan
- Trigger conditions: the focused `xtask` tests fail after the unrelated
  `win32.rs` blocker is resolved, or a headed baseline shows healthy panes
  below `SNAP_MID_LIT_PRESENCE_THRESHOLD`.
- Rollback procedure: revert this closure's edits to
  `xtask/src/desktop_regression/suites/post_resize_glitches.rs` and remove the
  closure-specific spec/task/note files, or `git revert` the eventual closure
  commit if it has been committed.
- Recovery time objective: under 10 minutes for a local revert.

### Specialist Reports

#### code-reviewer
- Correctness: implementation matches the closure spec ordering: resize and
  bottom stripe remain first, blank mid-pane is distinct, stale rows keep the
  existing upper bound.
- Readability: threshold naming and assertion messages are clear and local.
- Architecture: no new suite, dependency, diagnostic wiring, observe-mode, or
  foreground/occlusion changes.
- Security: no security-sensitive surface changed.
- Performance: adds constant-time checks only.
- Finding: no blocker for this closure.

#### security-auditor
- No secrets, auth, data access, environment, dependency, or external input
  handling changes.
- No security findings.

#### test-engineer
- Added focused tests for blank mid-pane classification, stale mid-pane
  classification, bottom-stripe precedence, and assertion behavior.
- `cargo test -p xtask desktop_regression` passes.
- `cargo build -p xtask` passes.
- `cargo fmt --check` is currently blocked by unrelated concurrent formatting
  diffs.
- Manual headed desktop suite skipped because it moves real windows and sends
  global input.
