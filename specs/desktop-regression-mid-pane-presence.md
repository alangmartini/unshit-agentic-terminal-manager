# Spec: Desktop Regression Mid-Pane Content Presence

## Objective
Make the existing `post-resize-glitches` Windows desktop regression suite fail
when the terminal pane is visually blank after Aero snap. The immediate users
are engineers and agents running the manual desktop regression harness to catch
terminal-manager renderer, layout, and terminal regressions.

Success means a post-snap screenshot with no lit terminal pane pixels reports
the first-bad signal `snap-mid-pane-blank`, while the existing stale-row and
bottom-statusbar pixel assertions keep their current behavior.

## Tech Stack
- Rust workspace package `xtask`.
- Existing desktop regression runner under `xtask/src/desktop_regression`.
- Existing pixel sampler and suite assertion helpers.
- No new non-Rust dependencies.

## Commands
- Focused tests: `cargo test -p xtask desktop_regression`
- Build: `cargo build -p xtask`
- Format check: `cargo fmt --check`
- Manual headed suite, not run automatically:
  `cargo xtask desktop-regression --suite post-resize-glitches`

## Project Structure
- `xtask/src/desktop_regression/suites/post_resize_glitches.rs`: suite logic,
  thresholds, failure classification, and focused unit tests.
- `specs/desktop-regression-mid-pane-presence.md`: this closure-specific spec.
- `tasks/desktop-regression-mid-pane-presence-plan.md`: implementation plan.
- `tasks/desktop-regression-mid-pane-presence-todo.md`: task checklist.
- `changelog.d/unreleased/`: release-note fragment if the change ships.

## Code Style
Keep the existing suite style: named constants near related thresholds, simple
boolean assertions, explicit failure signals, and focused unit tests.

```rust
let mid_present = ratios.mid_max_lit_ratio >= SNAP_MID_LIT_PRESENCE_THRESHOLD;
let mid_present_signal = classify_snap_failure(
    grew,
    ratios.bottom_lit_ratio,
    ratios.mid_max_lit_ratio,
    None,
);
assert_true(mid_present, "...", &mid_present_signal)?;
```

## Testing Strategy
- Add unit tests in `post_resize_glitches.rs` that exercise failure
  classification and assertion behavior without launching the desktop suite.
- Prove these cases:
  - blank mid-pane with healthy bottom stripe fails as `snap-mid-pane-blank`;
  - stale mid-pane still fails as `snap-mid-pane-stale-rows`;
  - bottom stripe failure keeps `snap-bottom-stripe-missing` precedence.
- Run `cargo test -p xtask desktop_regression` and `cargo build -p xtask`.
- Do not run the headed desktop suite unless explicitly requested, because it
  moves real windows, sends global input, and captures the live desktop.

## Boundaries
- Always: preserve black-box pixel assertions; keep the stale-row upper bound
  `mid_max_lit_ratio <= SNAP_MID_LIT_RATIO_THRESHOLD`.
- Always: use a stable first-bad signal `snap-mid-pane-blank`.
- Ask first: changing diagnostic producer wiring, observe-mode semantics,
  foreground or occlusion checks, CI behavior, or suite registration.
- Never: add a new suite, add CI, change root `SPEC.md`, or overwrite shared
  `tasks/plan.md` / `tasks/todo.md`.

## Threshold Calibration Note
The initial content-presence threshold is intentionally conservative and must
remain below the stale-row threshold. It is meant to distinguish a fully blank
pane (`mid_max_lit_ratio == 0.0`) from a pane with at least a few lit prompt
pixels after the suite sends `clear<Enter>`.

Healthy baseline values are not currently on file. Re-baseline
`SNAP_MID_LIT_PRESENCE_THRESHOLD` whenever the terminal theme, foreground
palette, antialiasing, sample geometry, or lit-pixel threshold changes.

## Success Criteria
- `assert_visual_ratios` fails fully blank mid-pane samples with
  `snap-mid-pane-blank`.
- Existing bottom stripe and stale-row classifications remain distinct.
- Focused `xtask` desktop-regression tests pass.
- The change is committed separately from unrelated worktree changes.

## Open Questions
- None for this closure item. Healthy desktop calibration should be captured by
  a future manual baseline run.
