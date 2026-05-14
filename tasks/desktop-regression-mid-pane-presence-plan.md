# Implementation Plan: Desktop Regression Mid-Pane Presence

## Overview
Add a lower-bound content-presence assertion to the existing
`post-resize-glitches` pixel checks. The change is a single vertical slice:
focused tests, threshold and classification logic, validation, review, and a
release-note fragment.

## Dependency Graph
`PixelSampleRatios.mid_max_lit_ratio`
-> `assert_visual_ratios`
-> `classify_snap_failure`
-> `SuiteError.first_bad_signal`
-> `results.json` failure signal.

The lower bound depends only on the sampled ratio and existing assertion helper.
No diagnostic producer, observe-mode, foreground, or occlusion code is required.

## Architecture Decisions
- Add a separate `SNAP_MID_LIT_PRESENCE_THRESHOLD` constant instead of reusing
  the stale-row threshold; lower and upper bounds express different failures.
- Assert bottom stripe first, mid-pane presence second, stale rows third. This
  preserves the existing bottom failure precedence and keeps stale-row behavior.
- Keep calibration guidance close to the threshold in code and mirrored in the
  spec because no healthy baseline artifact exists yet.

## Task List

### Phase 1: Behavior Guard
- [x] Task 1: Add focused failing tests for pixel failure classification.

**Acceptance criteria:**
- [ ] Blank mid-pane with healthy bottom stripe expects `snap-mid-pane-blank`.
- [ ] Stale mid-pane expects `snap-mid-pane-stale-rows`.
- [ ] Bottom stripe failure expects `snap-bottom-stripe-missing`.

**Verification:**
- [x] Red run attempted: `cargo test -p xtask desktop_regression`.
- [ ] Red run reached the new expectation.

Note: the initial red run was blocked before reaching the new expectation by
concurrent `win32.rs` occlusion work. After that worker's compile blocker was
resolved, the final green run covered the new tests.

**Dependencies:** None.

**Files likely touched:**
- `xtask/src/desktop_regression/suites/post_resize_glitches.rs`

**Estimated scope:** Small.

### Phase 2: Implementation
- [x] Task 2: Add the mid-pane content-presence threshold and assertion.

**Acceptance criteria:**
- [ ] `assert_visual_ratios` keeps the existing upper-bound stale-row check.
- [ ] Fully blank `mid_max_lit_ratio == 0.0` fails with `snap-mid-pane-blank`.
- [ ] Calibration note documents the conservative threshold and re-baseline
      requirement.

**Verification:**
- [x] Green run: `cargo test -p xtask desktop_regression`.
- [x] Build succeeds: `cargo build -p xtask`.
- [ ] Format check succeeds: `cargo fmt --check`.

Note: an earlier format check passed before concurrent worker edits landed.
The final format check is blocked by unrelated formatting in
`crates/terminal-manager-diagnostics/tests/serialization.rs`,
`src/diagnostics/snapshot.rs`, `xtask/src/desktop_regression/win32.rs`, and
snap-composition hunks in `post_resize_glitches.rs`.

**Dependencies:** Task 1.

**Files likely touched:**
- `xtask/src/desktop_regression/suites/post_resize_glitches.rs`

**Estimated scope:** Small.

## Checkpoint: Complete
- [ ] All acceptance criteria met.
- [x] Code review and simplification pass find no implementation blockers.
- [x] Ship decision recorded as GO with formatter caveat.
- [x] Changelog fragment created.

## Risks and Mitigations
| Risk | Impact | Mitigation |
|---|---|---|
| Theme or palette changes reduce lit prompt pixels below the threshold. | False failure in manual desktop runs. | Keep the threshold below stale-row detection and document re-baselining on theme or palette changes. |
| Blank and bottom-stripe failures overlap. | Less stable first-bad signal. | Preserve bottom assertion order and test precedence. |
| Other workers modify nearby suite logic. | Merge conflict or accidental overwrite. | Scope edits to the owned suite file and closure-specific artifacts only. |

## Open Questions
- None. Headed desktop validation remains manual for this closure.
