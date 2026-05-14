# Implementation Plan: Desktop Regression Basic Step Snapshots

## Overview
Promote diagnostic step snapshots from `--observe full` to both observed modes
while keeping the heavier deterministic/marker/invariant workflow full-only.
Document the observe-mode split and validate the mode predicate with focused
unit tests.

## Dependency Graph
`ObserveMode` contract
-> observability helper predicate
-> suite calls that receive `pre-snap` / `post-snap` snapshots
-> README/runbook documentation
-> validation and ship review

## Architecture Decisions
- Keep the change in `observability.rs` so all existing suite call sites pick
  up the behavior without per-suite edits.
- Add a pure helper for the step-snapshot mode predicate so it can be tested
  without a diagnostic pipe.
- Leave producer-side diagnostic data wiring and desktop assertion thresholds
  untouched.

## Task List

### Phase 1: Helper Contract

#### Task 1: Make Basic Eligible For Step Snapshots
**Description:** Add focused tests for the step-snapshot observe-mode predicate,
then update `capture_step_snapshot` to use the predicate.

**Acceptance criteria:**
- [x] Basic and full modes are eligible for step snapshots.
- [x] Off mode remains ineligible.
- [x] Missing diagnostics still return `Ok(None)`.
- [x] Full-only behavior outside `capture_step_snapshot` remains unchanged.

**Verification:**
- [x] `cargo test -p xtask desktop_regression`
- [x] `cargo fmt --check`

**Dependencies:** None

**Files likely touched:**
- `xtask/src/desktop_regression/suites/observability.rs`

**Estimated scope:** XS

### Checkpoint: Helper Contract
- [x] Predicate test passes.
- [x] No suite call-site changes required.

### Phase 2: Documentation

#### Task 2: Update Observe-Mode Documentation
**Description:** Update the desktop-regression README and debugging runbook to
state that basic mode includes step snapshots, while full mode adds the heavier
full-only diagnostic workflow.

**Acceptance criteria:**
- [x] `tests/windows/desktop-regression/README.md` documents basic step snapshots.
- [x] `docs/desktop-regression-debugging.md` documents basic step snapshots.
- [x] Docs do not claim producer-side events/fields were wired.

**Verification:**
- [x] `cargo test -p xtask desktop_regression`

**Dependencies:** Task 1

**Files likely touched:**
- `tests/windows/desktop-regression/README.md`
- `docs/desktop-regression-debugging.md`

**Estimated scope:** XS

### Checkpoint: Complete
- [x] All acceptance criteria met.
- [x] Ship review complete.
- [x] Changelog artifact created if the repository convention supports it.

## Risks and Mitigations
| Risk | Impact | Mitigation |
|---|---|---|
| Basic runs fail if diagnostic snapshot capability is unavailable | Medium | Existing basic capability validation already requires `snapshot`; keep behavior aligned with that contract. |
| Full-only behavior accidentally broadens | Medium | Keep existing `ObserveMode::Full` guards for deterministic mode, step markers, invariants, and clear-step. |
| Parallel worker changes conflict | Medium | Touch only scoped files and inspect status before committing. |

## Open Questions
None.
