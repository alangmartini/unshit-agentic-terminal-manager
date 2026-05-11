# Spec: Desktop Regression Snap Composition Guard

## Objective

Make the existing `post-resize-glitches` Windows desktop regression suite fail
before screenshot capture when the sampled desktop would be contaminated by
Task View or another foreground/overlapping window after `Win+Left`.

Target users are engineers and agents running the manual Windows desktop
regression harness.

## Tech Stack

- Rust `xtask` desktop regression runner.
- Existing `winapi` dependency for Windows desktop APIs.
- Non-Windows builds must compile through stubbed helpers.

## Commands

- Focused tests: `cargo test -p xtask desktop_regression`
- Build check: `cargo check -p xtask`
- Manual headed run: `cargo xtask desktop-regression --suite post-resize-glitches`

## Project Structure

- `xtask/src/desktop_regression/win32.rs`: Win32 wrappers, non-Windows stubs,
  and pure rectangle/occlusion decision helpers.
- `xtask/src/desktop_regression/suites/post_resize_glitches.rs`: suite call
  site after post-snap settling and before screenshot capture/pixel sampling.
- `specs/desktop-regression-snap-composition.md`: this closure-specific spec.
- `tasks/desktop-regression-snap-composition-plan.md`: implementation plan.
- `tasks/desktop-regression-snap-composition-todo.md`: task checklist.

## Code Style

Keep Win32 calls behind small functions and keep pure decisions separately
testable:

```rust
if let Some(occluder) = first_visible_occluder(post_rect, z_order_windows) {
    return Err(SuiteError::assertion(
        format!("post-snap window is obscured by {occluder:?}"),
        "snap-window-occluded",
    ));
}
```

## Testing Strategy

- Add unit tests for pure rectangle overlap and occlusion decision logic.
- Add suite-level unit coverage for mapping snap-composition failures to
  `SuiteError` first-bad signals where practical.
- Run `cargo test -p xtask desktop_regression`.
- Run `cargo check -p xtask`.
- Do not require a real HWND for automated tests.

## Boundaries

- Always preserve real desktop black-box assertions.
- Always assert foreground and composition after post-snap settle and before
  post-snap screenshot capture.
- Always keep non-Windows stubs compiling.
- Ask first before adding dependencies, CI, or a new suite.
- Never change mid-pane lit thresholds, diagnostic producer wiring, or
  observe-mode snapshot semantics in this closure item.
- Never overwrite root `SPEC.md`, `tasks/plan.md`, or `tasks/todo.md`.

## Success Criteria

- `post-resize-glitches` fails with first-bad signal
  `snap-foreground-stolen` if `GetForegroundWindow()` is not the app window.
- `post-resize-glitches` fails with first-bad signal
  `snap-window-occluded` if a visible, non-owned top-level window higher in
  Z-order overlaps the post-snap app rect.
- If implemented, stuck Win/Ctrl/Shift/Alt after `send_win_left` fails with
  `snap-stuck-modifier`.
- Pure helper tests cover edge-touching rectangles, real overlaps, owned
  windows, hidden windows, and first occluder selection.

## Open Questions

- The headed desktop run remains manual and is expected to be skipped in this
  automated closure pass unless explicitly requested.
