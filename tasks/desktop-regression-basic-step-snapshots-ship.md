# Ship Decision: Desktop Regression Basic Step Snapshots

## Ship Decision: GO

### Blockers
- None.

### Recommended Fixes
- None for this closure item.

### Acknowledged Risks
- The shared live worktree has unrelated uncommitted desktop-regression edits
  that currently block `cargo test -p xtask desktop_regression`; validation for
  this commit was run in a clean detached worktree at `a3441e9`.
- Headed desktop regression runs were skipped because they move real windows,
  send global input, and are documented as manual.

### Rollback Plan
- Trigger conditions: basic observed desktop-regression runs begin failing on
  snapshot capture despite a successful diagnostic handshake, or artifact volume
  from step snapshots is judged unacceptable.
- Rollback procedure: `git revert a3441e9`, then rerun
  `cargo test -p xtask desktop_regression` in a clean worktree.
- Recovery time objective: under 10 minutes for a local revert and focused
  validation.

### Specialist Reports

#### code-reviewer
- Correctness: `capture_step_snapshot` now uses a dedicated observe-mode
  predicate that returns true for basic and full and false for off. The existing
  full-only guards for deterministic mode, step markers, invariant evaluation,
  and clear-step behavior are unchanged.
- Readability: the helper names the policy clearly and keeps call-site logic
  short.
- Architecture: the behavior stays in the shared observability helper, so
  existing suite call sites receive snapshots without suite-specific changes.
- Security: no secrets, auth, data access, environment, dependency, or external
  input handling changes.
- Performance: adds no loops or unbounded work; the behavior intentionally adds
  one diagnostic snapshot request and JSON artifact write where suites already
  ask for step snapshots.
- Findings: none.

#### security-auditor
- No credential, token, authorization, dependency, or network exposure changes.
- Snapshot capture remains gated behind explicit diagnostics-enabled observe
  modes. Basic already required snapshot capability during diagnostic handshake.
- Risk noted: future producer-side terminal buffer inclusion could make basic
  step snapshots contain more sensitive terminal state. Current scope does not
  wire those producer fields.
- Findings: none.

#### test-engineer
- Added focused unit coverage for the observe-mode predicate and for the
  no-diagnostics basic-mode skip path without requiring a live app.
- Verified in a clean detached worktree at `a3441e9`:
  `cargo fmt --check`, `cargo test -p xtask desktop_regression`,
  `cargo build -p xtask`.
- Manual headed desktop suite skipped by design for this closure item.
- Findings: none.
