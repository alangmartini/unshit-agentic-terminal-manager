### Added

- Vblank-anchored frame pacing (scroll-smoothness spec Phase 4): on a vsync-paced surface the renderer's blocking swapchain acquire now anchors the paint loop to the display's refresh clock, so animation frames land one-per-refresh instead of being driven by a wall-clock timer that beats against scanout (the old unsynced ~125Hz producer against a 120Hz scanout dropped or doubled a frame every ~25 frames). The window swapchain now prefers `Fifo` (guaranteed on Vulkan) over Mailbox/Immediate; surfaces without a blocking present mode fall back to true-period timer pacing.
- `UNSHIT_FRAME_LATENCY` environment variable (accepts `1` or `2`) to A/B the swapchain's maximum frame latency without a rebuild; the default is `1` (smallest queue, lowest input-to-photon latency). The per-frame swapchain-acquire wait is measured and subtracted from work-time metrics so vblank blocking never masquerades as render cost.

### Changed

- The frame pacer no longer emulates vsync or "timer-compensates" 120Hz down to 8ms; it now reports the display's true refresh period (e.g. 8.333ms at 120Hz, 16.666ms at 60Hz) and survives only as the metrics floor and the Timer-fallback redraw coalescer. Refresh-rate reports below 10Hz are treated as driver garbage and fall back to the 8ms default.
- The animation waker is now the Timer-fallback tick source only: on the default vsync-paced path its thread is never spawned (the blocking acquire is the tick), and when it is used its interval is the display's true refresh period.

### Fixed

- Swapchain acquire failures follow an explicit, unit-tested recovery policy: `Lost` always reconfigures, `Outdated` reconfigures at most once per episode and never while the window is minimized (preventing a reconfigure storm and an unvalidated stale-extent submit on minimized Vulkan windows), and timeouts/other errors drop the frame without touching the surface.
