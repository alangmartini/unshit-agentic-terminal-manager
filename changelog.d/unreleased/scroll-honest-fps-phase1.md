### Added

- Honest presentation-cadence metrics (scroll-smoothness spec Phase 1): the FPS overlay now reports fps as paints within the trailing second instead of `1/work_time`, with new `int p50/p95/p99/max` present-interval rows and a `dropped` counter (intervals > 1.5x the display period); the work-time number keeps its own `work` row and the CPU encode time is labeled `encode` instead of `gpu`. A second `[FRAME-INTERVAL] intervals=... dropped=...` log line emits cadence quantiles once per second alongside the existing work-time `[FRAME]` line. Idle cadence breaks (paint gaps of 250ms or more, e.g. the 500ms cursor-blink repaint) are excluded so an idle session does not fabricate jank.

### Changed

- Bench report JSON: `fps_mean` renamed to `paints_per_sec_mean`; new `interval_p50/p95/p99/max_ms`, `interval_stddev_ms`, `judder_ratio`, and a 0.5ms-bucket `interval_histogram` of present intervals.
- A visible FPS overlay now requests rebuilds at most ~4Hz instead of every painted frame, so enabling it no longer ejects smooth scrolling from the fast-paint path on every frame.
