# Implementation Plan: Desktop Regression Snap Composition Guard

## Overview

Add a narrow post-snap guard to the existing `post-resize-glitches` suite. The
guard uses reusable Win32 helpers to verify that the terminal-manager window is
still foreground, no modifier key is stuck, and no visible non-owned top-level
window above it overlaps its rect before screenshots and pixel samples are
captured.

## Dependency Graph

`DesktopRect` overlap logic
    -> pure occlusion decision helper
    -> Win32 foreground/modifier/Z-order wrappers
    -> `post_resize_glitches` assertion mapping
    -> focused tests and validation

## Architecture Decisions

- Keep all platform API calls in `win32.rs` and expose a single
  `assert_snap_ready_for_capture`-style helper to the suite.
- Model foreground, stuck-modifier, and occlusion as an enum so the suite can
  produce stable first-bad signals without string parsing.
- Keep occlusion filtering pure and testable with lightweight window facts
  rather than requiring HWNDs in tests.

## Task List

### Phase 1: Pure Occlusion Logic

- [x] Task 1: Add rectangle overlap and occlusion decision tests/helpers.
  - Acceptance: Helpers identify overlapping visible non-owned windows above
    the target and ignore hidden, owned, self, below-target, and edge-touching
    windows.
  - Verification: `cargo test -p xtask desktop_regression::win32`
  - Dependencies: None
  - Files likely touched: `xtask/src/desktop_regression/win32.rs`
  - Estimated scope: S

### Checkpoint: Foundation

- [x] Focused helper tests fail before implementation and pass after.

### Phase 2: Win32 Snap Guard

- [x] Task 2: Add Windows foreground, modifier, and Z-order composition helper.
  - Acceptance: Windows implementation checks foreground equality, stuck
    modifiers, and visible non-owned overlapping windows higher in Z-order;
    non-Windows stubs return the existing unsupported error style.
  - Verification: `cargo check -p xtask`
  - Dependencies: Task 1
  - Files likely touched: `xtask/src/desktop_regression/win32.rs`
  - Estimated scope: M

### Phase 3: Suite Integration

- [x] Task 3: Call the snap guard before post-snap screenshot capture.
  - Acceptance: `post-resize-glitches` maps failures to
    `snap-foreground-stolen`, `snap-stuck-modifier`, and
    `snap-window-occluded` before sampling post-snap pixels.
  - Verification: `cargo test -p xtask desktop_regression`
  - Dependencies: Task 2
  - Files likely touched:
    `xtask/src/desktop_regression/suites/post_resize_glitches.rs`
  - Estimated scope: S

### Checkpoint: Complete

- [x] `cargo test -p xtask desktop_regression`
- [x] `cargo check -p xtask`
- [x] Ship review completed.
- [x] Changelog fragment created.

## Risks and Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| Z-order filtering flags owned/tool windows as occluders | False failures | Ignore owned top-level windows and the target itself. |
| `GetForegroundWindow` briefly lags after snap | Flaky failure | Check only after the existing 1500 ms settle sleep. |
| Non-Windows compile break | Local developer friction | Keep stubs for all new public helpers. |

## Open Questions

- None for implementation; headed run remains manual.
