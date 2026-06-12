### Added

- Animated terminal scrolling (scroll-smoothness spec Phase 3): wheel notches now ease the scrollback view over ~180ms with the same browser-validated curve as the settings page, retargeting in-flight on new notches, instead of teleporting whole rows per event. Content renders with sub-row (device-pixel-snapped) precision via a one-row overscan snapshot and a paint-time translation, so motion is continuous rather than row-quantized.
- Scrolled-back viewport anchoring: streaming PTY output no longer snaps the view to the live bottom while reading scrollback; the view (and any in-flight scroll animation) shifts with scrollback growth, including at-capacity eviction. Entering or leaving the alternate screen (full-screen TUIs) still snaps to live.

### Changed

- One persistent deadline-extended animation waker thread replaces the per-wheel-notch waker threads; terminal scroll animation and container smooth scroll tick from the same shared motion module (`scroll_motion`), sampled via injectable timestamps in preparation for vblank-anchored pacing (Phase 4).
- The experimental `grid-fragment-shader` path no longer applies to terminal grids (no overscan support); terminal grids render through the standard cell emitter unconditionally.
