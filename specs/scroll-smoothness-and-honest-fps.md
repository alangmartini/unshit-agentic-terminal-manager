# SPEC: Scroll Smoothness and Honest FPS Measurement

> Status: Draft, awaiting confirmation
> Owner: Alan Martini
> Priority: P0 (direct successor to `specs/120fps.md`; that spec's instrument is the bug here)
> Date: 2026-06-12
> Related code: `src/ui/fps_overlay.rs`, `src/ui/terminal_grid.rs`, `src/terminal/mod.rs`, `src/bench.rs`, `crates/unshit-framework/crates/unshit-app/src/app.rs`, `crates/unshit-framework/crates/unshit-app/src/frame_pacer.rs`, `crates/unshit-framework/crates/unshit-app/src/frame_probe.rs`, `crates/unshit-framework/crates/unshit-renderer/src/gpu.rs`, `crates/unshit-framework/crates/unshit-renderer/src/batch.rs`, `xtask/src/desktop_regression/suites/settings_scroll.rs`
> Provenance: multi-agent audit (2026-06-11), 43 findings confirmed under adversarial code verification, 0 refuted.

## 1. Objective

Two coupled problems, one user-visible symptom: the FPS overlay reads ~400fps while wheel scrolling visibly stutters and feels worse than VS Code on the same machine (Samsung Odyssey G6, currently driven at 120Hz, AMD Radeon 890M, Windows 11).

1. **The fps number is not a frame rate.** The overlay computes `1e6 / total_us` where `total_us` is the CPU work time of one frame, so "400 fps" means "2.5ms of work", not "400 frames reached the screen". Actual paints are capped at ~125/s by an 8ms timer and the display shows at most 120/s. Nothing anywhere in the codebase measures presentation cadence, so the real jank is structurally invisible to every existing metric, log line, bench, and regression suite.
2. **Scrolling is not smooth because motion is wrong, not because rendering is slow.** The terminal grid converts each wheel notch into an instant whole-row teleport (about 7 rows at once), bypassing the framework's own eased smooth-scroll animator. Where the animator does run (settings, sidebar), frames are produced by a free-running 8ms timer beating against the 8.333ms scanout with vsync deliberately disabled, which drops roughly one frame every 200ms.

This spec makes the measurement honest first, then makes scrolling actually smooth, then locks both in with regression gates that would have caught the original complaint.

### Why this matters

`specs/120fps.md` is P0 and its entire acceptance matrix is gated on the FPS overlay, which currently measures the wrong quantity in the flattering direction. Every gate can pass on a janky build. Until the instrument is fixed, no perf claim from that spec is trustworthy. And scroll feel is the single most frequent interaction in a terminal; VS Code sets the bar on this exact machine.

### Non-goals

* Raising the panel to 500Hz is an environment task, not a code fix (see Phase 0). The app must be smooth at 120Hz first; a higher refresh rate shrinks pacing error but cannot fix row-teleport scrolling.
* No render-quality reductions (MSAA, glyph hinting) to hit numbers, per `specs/120fps.md` boundaries.

## 2. Acceptance Criteria

### Hard gates (must pass to ship)

| # | Gate | Metric | Target |
|---|---|---|---|
| H1 | Overlay fps is honest | Overlay "fps" vs PresentMon Displayed FPS during sustained scroll | Within 10%, and never exceeds the active refresh rate |
| H2 | Present cadence is flat during scroll | Present-to-present interval p99 during a 3s wheel burst (10 notches/s) | <= 1.5x display period (12.5ms at 120Hz) |
| H3 | No dropped-frame beat | Fraction of presents never displayed (PresentMon DisplayedTime=NA) during scroll | < 2% |
| H4 | Wheel motion is animated | Distinct presented scroll positions per wheel notch | >= 12 frames spread over 100-200ms, eased, retargets in-flight |
| H5 | Scroll distance is proportional | N notches scroll exactly N x configured px (no per-event ceil amplification); touchpad px deltas tracked 1:1 with fractional carry | Exact, property-tested |
| H6 | Sub-row precision | Terminal content renders at fractional row offsets during animation | Visible sub-row positions, no whole-cell snapping mid-animation |
| H7 | Event loop never blocks | `std::thread::sleep` on the winit event-loop thread | Zero call sites |
| H8 | Input-to-photon during scroll | Wheel event to first changed present | <= 2 display periods + 2ms |
| H9 | No metric regression | All `specs/120fps.md` hard gates re-expressed as work-time stats | Still pass |

### Soft gates (graceful degradation)

| # | Gate | Target |
|---|---|---|
| S1 | At 240/500Hz panel modes, paints track the panel cadence or degrade coherently to an integer fraction (half rate), never a beat | Verified once Phase 4 lands |
| S2 | Settings/sidebar animated scroll meets H2/H3 with the same thresholds | Same burst protocol |
| S3 | Scrolling during live PTY output does not visibly shift the viewport between paints | Manual + suite check |

### Canonical repro

1. Launch release build, one pane, `cat` a large file to fill scrollback.
2. Toggle the FPS overlay (`Ctrl+Shift+F`). Note the fps reading while idle and while scrolling.
3. Roll the wheel up 10 notches over ~1s, then down.
4. Today: overlay reads 300-400 "fps"; motion advances in ~7-row jumps at wheel-event rate with periodic hitches. After this spec: overlay reads <= 120, motion is continuously eased with sub-row offsets, no hitches.

External ground truth for every gate: Intel PresentMon capturing `MsBetweenPresents`, `MsBetweenDisplayChange`, `PresentMode`, `GPUBusy` during the repro.

## 3. Diagnosis Summary (all confirmed by adversarial code verification)

### Measurement defects

1. **fps = 1 / work time.** `current_fps()` inverts the last `total_us` sample (`src/ui/fps_overlay.rs:97`); `total_us = frame_start.elapsed()` (`app.rs:892`, `app.rs:3504`). It measures CPU build cost (tree, style, layout, batch, encode), never frame-to-frame interval.
2. **The correct implementation is dead code.** `FpsOverlayState.last_frame_at` is documented as "used to compute the window-based current fps", written every frame (`fps_overlay.rs:73`), and never read. The unit test `current_fps_inverts_last_sample_in_us` (`fps_overlay.rs:344`) codifies the wrong definition.
3. **No present-interval metric exists anywhere.** `FrameMetrics` (`app.rs:281`) has no interval field; `FrameProbe`, the `[FRAME]` log lines, `src/bench.rs` percentiles (`bench.rs:139`), and the overlay quantiles are all work-time. A frame that wakes 25ms late still records "2.5ms".
4. **GPU time is unmeasured.** `gpu.render()` ends with non-blocking `queue.submit` + `present()` (`gpu.rs:1374`, `gpu.rs:1392`); all render passes have `timestamp_writes: None`. `gpu_render_us` is CPU encode time only. 4x MSAA at 2560x1440 on an iGPU could exceed the frame budget invisibly.
5. **Two contradicting fps definitions ship today.** The window title computes paints/sec (`app.rs:3551`, would read ~125 during scroll) while the overlay reads 1/work (~400); the title counter also stalls on the smooth-scroll fast path (`app.rs:911` increments, no rollover block).
6. **Observer effects.** A visible overlay sends `RequestRebuild` every frame (`src/main.rs:1069`), forcing the heavy rebuild path and disabling the fast-paint path it is trying to measure (`app.rs:807`).
7. **`bench.rs` fps_mean counts submitted paints**, not displayed frames, and averages away cadence variance (`bench.rs:243`).

### Scroll-path defects (dominant cause of perceived jank)

8. **Terminal wheel scroll is an instant multi-row teleport.** The grid's handler converts the notch's ~150px to `ceil(px / cell_h)` whole rows applied in one frame (`src/ui/terminal_grid.rs:587`); `scroll_offset` is a `usize` of whole lines (`src/terminal/mod.rs:654`). Roughly 7 rows per notch at default font and 150% scale. Motion therefore updates at wheel-event rate (10-30Hz) in row-sized steps regardless of frame rate. VS Code eases the same notch over 125ms with pixel precision; this gap alone explains "worse than VS Code at 400fps".
9. **Round-away-from-zero per event, no fractional carry** ("minimum 1 line per notch", `terminal_grid.rs:594`): precision touchpads get ~5x amplification quantized to rows; every notch over-travels up to one row.
10. **Sub-notch LineDelta amplified 3x.** `normalize_wheel_line_delta` divides by 3.0 only when |delta| >= 3.0 (`app.rs:634`); high-resolution wheels emitting sub-notch events scroll up to 3x farther per detent, compounding with finding 9.
11. **The framework already has the cure and the terminal bypasses it.** `SmoothScroll` (eased, velocity-continuous retargeting, `app.rs:531-572`, `app.rs:685-734`) drives settings/sidebar containers, and the renderer supports fractional scroll offsets (`batch.rs:1541`). The terminal pane's `overflow` container has zero scrollable height so the animator no-ops (`app.rs:699`) and the raw handler teleports instead.
12. **Every consumed wheel event sets global `needs_rebuild`** (`app.rs:2907`), repainting terminal scroll through the heaviest path and kicking concurrent animations off the fast path.

### Pacing and presentation defects

13. **Presentation is never vblank-anchored.** `choose_present_mode` prefers Mailbox, then Immediate, then AutoNoVsync, never Fifo (`gpu.rs:117`), with a unit test enforcing the avoidance (`gpu.rs:2136`). Backend is forced Vulkan on Windows (`gpu.rs:87`). Frame production is open-loop CPU timers with zero display-clock feedback; `frame_pacer.rs:4` openly admits vsync is "emulated with a coalescing timer".
14. **8.000ms pacing against 8.333ms scanout.** `TIMER_COMPENSATED_120HZ_INTERVAL` clamps the 120Hz-derived interval to 8ms (`frame_pacer.rs:55`, `frame_pacer.rs:100`). A ~125Hz producer against a 120Hz consumer drifts 0.333ms/frame: one frame dropped or doubled every ~25 frames, roughly 5 hitches/second, made irregular by timer wake jitter. Timer resolution is NOT the problem (winit 0.31 WaitUntil and Rust 1.95 sleep both use high-resolution waitable timers); the missing piece is a vblank clock.
15. **The wheel-scroll path bypasses even the pacer.** Active smooth scrolls paint via `about_to_wait` with a hard-coded `SMOOTH_SCROLL_WAKE_INTERVAL = 8ms` (`app.rs:494`) and `std::thread::sleep` ON THE EVENT-LOOP THREAD (`app.rs:3597`), stalling input dispatch up to 8ms mid-interaction; `ControlFlow::Poll`; pacer explicitly skipped (`app.rs:3006`). The 8ms constant ignores the monitor, so even a 500Hz panel would scroll at 125Hz.
16. **Animations are sampled at CPU wake time, not display time** (`app.rs:788`): wake jitter becomes spatial velocity noise, and a beat slip advances the displayed scroll curve 16ms in one 8.33ms refresh (2x velocity spike).
17. **One waker thread spawned per wheel notch** (`app.rs:2834`, `app.rs:511-528`), each sending wakes every 8ms; fast wheel spins stack ~15 threads of mid-frame wakeups.
18. **Prior art is unanimous.** VS Code/Chromium, Zed (DwmFlush then DCompositionWaitForCompositorClock), Ghostty, Alacritty, WezTerm all pace exactly one frame per display refresh from a vsync/compositor event and animate wheel input. Ghostty's 8ms timer, cited by `frame_pacer.rs:32` as precedent, coalesces work but presents vsynced; the citation transplanted the timer without the anchor.

### Verification gaps

19. **The regression battery cannot fail on this complaint.** The settings-scroll suite's burst phase asserts only a ~24fps mean floor (`settings_scroll.rs:41`, frame counter delta over ~330ms); `observed_fps` is computed but never asserted; the 18ms max-sample-gap check (`settings_scroll.rs:40`) covers only the single-tick phase and permits two consecutive dropped 120Hz frames; the fps-overlay suite asserts only that counters advanced (`settings_scroll.rs:289`).
20. **`specs/120fps.md` enforcement artifacts were never built**: no `benches/frame_time.rs`, no `tests/fixtures/perf_baseline.json`, no CI comparison. `src/bench.rs` is report-only.

## 4. Phased Plan

Each phase ends with a measurement. Phases 1 and 2 are independent and can land in either order; Phase 3 depends on 2; Phase 4 is independent of 2/3 but its acceptance needs Phase 1's metrics; Phase 5 needs 1 and 4.

### Phase 0: Ground truth and environment audit (no code changes beyond logging)

Goal: pin down the runtime facts the audit could not see from source, so fixes target reality.

1. Capture the startup log of selected backend and present mode (already logged at `gpu.rs:314-318`). On AMD Vulkan, Mailbox may be unavailable, meaning the app actually runs Immediate; the difference changes Phase 4's option ranking.
2. Run the canonical repro under Intel PresentMon. Record `MsBetweenPresents`, `MsBetweenDisplayChange` (distribution, not mean), `PresentMode` (Composed Flip vs Hardware Independent Flip; MPO on AMD iGPU is a known stutter source), `GPUBusy`. This is the baseline table for gates H1-H3.
3. Record a wheel-input trace with the existing hook (`app.rs:2752` logs dx/dy/time_ms): device event rate, LineDelta vs PixelDelta, sub-notch fragmentation. Determines how much findings 9/10 contribute on the user's actual mouse.
4. Environment checks, each a cheap A/B: why is the 500Hz panel at 120Hz (cable/DSC, Parsec, or choice); disable the Parsec Virtual Display Adapter and retest; check VRR/FreeSync, Windows 11 "Optimizations for windowed games", AMD Adrenalin Enhanced Sync/Anti-Lag overrides.
5. Discriminating experiment: temporarily set the panel to 240Hz. If scroll jank is unchanged, row-teleport dominates (expected); if it improves a lot, pacing dominates and Phase 4 moves up.
6. GPU cost: read GPUBusy from the PresentMon trace during the repro; if > 6ms, schedule wgpu `TIMESTAMP_QUERY` instrumentation into Phase 1 scope.

Exit criteria: a filled baseline table in this spec (PresentMon distributions, actual present mode, wheel-device profile, environment findings).

#### Baseline measurements (recorded 2026-06-12, Phase 0)

| Fact | Value | Implication |
|---|---|---|
| Backend / adapter | Vulkan, AMD Radeon 890M, driver 26.3.1 (LLPC) | as designed (`gpu.rs:87`) |
| Surface present modes | `[Immediate, Fifo, FifoRelaxed]`, selected **Immediate** | **No Mailbox on this hardware.** Open Question 1 resolved: the app runs fully unsynced Immediate presents; DWM samples the latest frame at each compose. Fifo is guaranteed available, de-risking Phase 4 Option A |
| Display modes at 2560x1440 | 48/60/75/100/**120**/240/500 Hz available on the current HDMI link (VideoOutputTechnology=5) | 120Hz is a Windows settings choice, not a link limit. Open Question 2 partially resolved; 240/500Hz reachable for S1 verification without recabling |
| winit-reported refresh | 120000 mHz, pacer interval 8.0ms | the `TIMER_COMPENSATED_120HZ_INTERVAL` clamp path is live |
| Parsec Virtual Display Adapter | Installed, Status OK, not presenting a desktop (single active display) | low risk as primary-monitor confounder; A/B deferred |
| Bench baseline (dir-loop, 10s, release) | frames=1256, paints/sec=**124.77**, work p50=1.139ms p95=1.547ms p99=1.806ms max=2.292ms, gpu-encode avg 0.61ms | paint cadence is exactly the 8ms timer (125Hz) against a 120Hz panel: ~4.8 undisplayable frames/sec, matching the predicted beat. 1/work would read ~880 "fps", a ~7x inflation over true cadence |
| PresentMon | Blocked: shell not elevated (ETW capture needs admin) | external ground truth deferred to an elevated session; Phase 1 in-app interval metrics are the primary instrument until then |
| Wheel-device trace | Deferred to the Phase 5 suite run (win32 synthesized input); default Windows wheel assumption (3 lines/notch) stands | findings 9/10 sizing unaffected for a standard wheel |
| GPU execution cost | Unmeasured (GPUBusy needs PresentMon; no timestamp queries yet) | decision on `TIMESTAMP_QUERY` deferred until interval metrics show whether cadence holds 120 |
| 240Hz discriminating experiment | Deferred to Phase 4 S1 (mode switch automatable via `ChangeDisplaySettingsEx`) | n/a |

Verdict: diagnosis confirmed live. Present mode is Immediate (not Mailbox), the 8ms-vs-8.333ms producer/consumer mismatch is measured (124.77 paints/sec), and the overlay's work-time inversion would overstate fps ~7x on this workload. Proceed to Phase 1.

### Phase 1: Honest metrics

Goal: every number labeled fps is a presentation-cadence number; work time keeps its own label.

1. Implement what `last_frame_at` was designed for: keep a rolling window of frame timestamps in `FpsOverlayState::record`, report fps as frames within the trailing second (or 1/mean-interval). Relabel the existing 1/work number as `work cap` or drop it; keep `total_us` quantiles under `work`.
2. Add an interval ring next to the work ring: present-interval p50/p95/p99/max plus a dropped-interval counter (intervals > 1.5x display period), surfaced in the overlay and the once-per-second `[FRAME]` lines (`frame_probe.rs` learns a second channel or a second probe instance).
3. Add `present_interval_us` to `FrameMetrics` so bench and diagnostics inherit it. In `bench.rs`: interval histogram, `interval_p99_ms`, `interval_stddev_ms`, `judder_ratio`; rename `fps_mean` to `paints_per_sec_mean`.
4. Fix the observer effect: throttle overlay-driven `RequestRebuild` to <= 4Hz; verify enabling the overlay no longer disqualifies `can_fast_paint_smooth_scroll`.
5. Unify the window-title fps with the overlay definition; run the once-per-second rollover on both paint paths (fast path currently only increments).
6. Rewrite the test `current_fps_inverts_last_sample_in_us` to feed timestamps 8.33ms apart and assert ~120 regardless of `total_us`.
7. Relabel `gpu_render_us` as CPU encode time in the overlay; if Phase 0 step 6 flagged GPU cost, add `TIMESTAMP_QUERY` GPU timing as a separate stat.
8. Expose the last N frame timestamps in the diagnostics snapshot for the Phase 5 suite.

Exit criteria: during the canonical repro the overlay fps reads <= 120 and matches PresentMon Displayed FPS within 10% (gate H1); the overlay shows interval quantiles that visibly expose the beat (bimodal distribution) before Phase 4 fixes it. Note: until Phase 4 anchors pacing to vblank, the honest fps reads the paint rate (~125 at the 8ms timer), which exceeding the 120Hz refresh is itself the diagnostic signal; the full <= refresh check is Phase 4's acceptance.

#### Phase 1 results (recorded 2026-06-12)

Implemented across `app.rs` (shared `finalize_frame_metrics` epilogue for both paint paths, `present_interval_us` + `display_period_ns` in FrameMetrics, second `FrameProbe`, unified title-fps rollover), `fps_overlay.rs` (honest fps from a trailing-1s timestamp window, `int p50/p95/p99/max` + `dropped` rows, work row relabels, 4Hz rebuild throttle, dead `last_frame_at` removed), `frame_probe.rs` (`count_above_us`), `bench.rs` (interval quantiles, stddev, judder_ratio, 0.5ms-bucket histogram, `fps_mean` renamed `paints_per_sec_mean`). Idle cadence breaks (gaps >= 250ms, e.g. cursor-blink wakes) are zeroed at the source so idle sessions do not fabricate jank; the tradeoff (hangs > 250ms conflated with idle) is acceptable until Phase 4's vblank anchor. Workspace green: 3728 tests, 0 failed; clippy clean in touched files.

Live verification (dir-loop bench, release, 120Hz):

| Metric | Value | Reading |
|---|---|---|
| paints_per_sec_mean | 124.71 | 8ms timer producer confirmed against 120Hz consumer |
| interval p50 / p95 / p99 / max | 7.99 / 8.59 / 9.12 / 14.07 ms | cadence locked to the timer, not the 8.333ms display period; the gap IS the beat |
| interval stddev | 0.39 ms | wake jitter, matches the audit's ~0.5ms estimate |
| judder_ratio (> 1.5x period) | 0.16% | paint-side; display-side drops remain invisible until PresentMon or Fifo |
| histogram | unimodal at 7.5-8.5ms, outliers at ~14ms | missed-wake doubles visible |

Phase 4 acceptance becomes: interval p50 -> ~8.33ms, paints/s -> ~120, outlier buckets empty.

### Phase 2: Scroll input correctness (small diffs, immediate feel improvement)

Goal: scroll distance is exactly proportional to input; no quantization amplification.

1. Fractional accumulator in the terminal wheel handler: `acc += raw_lines; lines = acc.trunc(); acc -= lines`. Kill the per-event ceil/floor and the >= 1-row floor.
2. `normalize_wheel_line_delta`: always divide by the notch size; query `SPI_GETWHEELSCROLLLINES` instead of hard-coding 3.0. Sub-notch events scale proportionally.
3. Scoped invalidation for terminal scroll: paint-dirty the grid node instead of global `needs_rebuild = true` so wheel events stop forcing full rebuilds and stop ejecting concurrent animations from the fast path.

Exit criteria: gate H5 property tests pass (N notches = exactly N x configured px; fractional sequences sum exactly); wheel-trace replay from Phase 0 shows no amplification on the user's device.

#### Phase 2 results (recorded 2026-06-12)

All three steps landed, and step 3 (scoped invalidation) proved feasible-now rather than deferred:

1. `Terminal::scroll_view_by_lines` implements trunc-and-carry fractional accumulation (`src/terminal/mod.rs`); the wheel handler passes raw fractional lines with no per-event rounding and no >= 1-row floor. Carry is discarded on boundary clamp, on carry pinned into a boundary, on every snap-to-live path (PTY output, snapshot restore, ED 3), and non-finite deltas are rejected. Covered by 13 unit tests including a 200-step fixed-seed property walk (gate H5's test clause).
2. `normalize_wheel_line_delta` divides unconditionally by the OS notch size, queried per event (matching winit's per-event multiply, so mid-session settings changes stay in sync): `SPI_GETWHEELSCROLLLINES` for the vertical axis, `SPI_GETWHEELSCROLLCHARS` for the horizontal axis, `WHEEL_PAGESCROLL` sentinel and failure fall back to 3.0; non-Windows divides by 1.0. Behavior note: a Windows "wheel scroll lines" preference other than the default 3 is now neutralized by design (one detent = exactly `line_scroll_px`); the app's own scroll-speed setting is the speed knob, per gate H5 proportionality.
3. Scoped invalidation shipped as `ScrollGridPatch`: a Scroll handler may return a same-dimensions grid content patch; the dispatch applies it as a paint-only arena content swap (mirroring the reconciler's `grid_content_paint_only` classification) and no longer sets global `needs_rebuild`, so wheel-over-terminal no longer forces full rebuilds nor ejects smooth-scroll animations from the fast-paint path. Dimension mismatch or non-patch returns fall back to the legacy full rebuild.

Standard-wheel feel change: one notch was ceil(6.15) = 7 rows instantly; now trunc with carry = 6 rows on most notches, 7 on ~every 7th, long-run average exactly 6.15 rows/notch (proportional). Workspace green: 3746+ tests, 0 failed. Deferred per Phase 0: the wheel-trace replay clause of the exit criteria runs with the Phase 5 suite; the end-to-end dispatch-glue test also lands with Phase 5's terminal-scroll suite.

### Phase 3: Animated terminal scrolling (the browser feel)

Goal: a wheel notch produces motion, not a jump; sub-row precision end to end.

1. Per-pane f32 scroll position in pixels. Wheel deltas add to a target; the visible position eases toward it through the existing `SmoothScroll` curve (ease-out, ~125-180ms, velocity-continuous retarget on new notches, matching the settings-page tuning already validated against an Edge baseline).
2. Map the animated pixel position to `(whole_rows, remainder_px)` each frame. Step A: apply `whole_rows` via `scroll_view_up/down` per animation frame (motion spread over ~20 frames instead of 1). Step B: render the grid fragment translated by `remainder_px` (draw rows+1; the renderer's fractional offset support at `batch.rs:1541` is the mechanism), removing whole-cell snapping (gate H6).
3. Route scroll animation frames through the fast-paint path (depends on Phase 2 step 3).
4. Replace per-notch waker threads with one persistent animation clock whose deadline extends on retarget.
5. Define scrollback semantics during live PTY output (offset anchoring so the viewport does not shift between paints); cover with a suite case (gate S3).

Exit criteria: gate H4 passes (>= 12 distinct presented positions per notch); side-by-side manual comparison against VS Code judged acceptable by the user; settings-scroll suite still green.

### Phase 4: Vblank-anchored pacing and presentation

Goal: exactly one frame per display refresh while animating; cadence derived from the display, not Instant math.

1. Option A (default, lowest risk): select `PresentMode::Fifo` with `desired_maximum_frame_latency: 1..2`. `get_current_texture`/`present` then pace the loop at the real composition rhythm; the pacer demotes to a work-coalescing floor. Option B (if A's input latency disappoints): keep Mailbox, add the vsync hook `frame_pacer.rs:4` wishes for: a waker thread on `DCompositionWaitForCompositorClock` (Win11) falling back to `DwmFlush` (Win10), one paint scheduled per tick. Decide with PresentMon + latency data, document the decision here.
2. Delete `TIMER_COMPENSATED_120HZ_INTERVAL` and the 120Hz special case; derive intervals from the true period everywhere.
3. Delete the `std::thread::sleep` in `about_to_wait` (gate H7) and `SMOOTH_SCROLL_WAKE_INTERVAL`; collapse the four duplicated fast-paint call sites (about_to_wait, RedrawRequested, proxy_wake_up, slow-path reschedule) into one animation-tick path driven by the frame clock.
4. Sample animations at predicted present time (`last_present + period`) instead of `frame_start` (fixes finding 16).
5. Re-probe the monitor refresh rate on display-mode change, not only on `WindowEvent::Moved` (stale-interval bug after changing Hz in Windows settings).
6. Verify S1 at 240Hz (and 500Hz if the panel reaches it): paints track the panel or degrade to a coherent integer fraction.

Exit criteria: gates H2, H3, H8 pass under PresentMon (flat `MsBetweenDisplayChange` at ~8.33ms, < 2% dropped, latency within budget); the Phase 1 overlay interval histogram shows a single tight mode.

### Phase 5: Regression gates that would have caught this

Goal: a janky-at-high-reported-fps build fails CI/xtask.

1. Settings-scroll suite: assert the interval distribution during the 14-tick burst (not just a mean floor): interval p99 <= 1.5x display period, gap stddev <= 2ms, dropped ratio < 2%, using the Phase 1 diagnostics timestamp ring. Apply the max-gap check to the burst phase; tighten 18ms to period + scheduling epsilon (~10ms at 120Hz).
2. Add per-presented-frame scroll displacement variance (the most direct jank proxy without photon capture): stddev of per-frame scroll_y delta during constant-velocity burst below a threshold.
3. New terminal-scroll suite mirroring the settings suite: notch proportionality (H5), animation frame count (H4), sub-row offsets present (H6).
4. Resolve the `specs/120fps.md` enforcement debt: bless `src/bench.rs` as the harness, add a threshold-checking xtask wrapper that fails when work p99 or the new interval metrics regress beyond a committed `tests/fixtures/perf_baseline.json`. (Amend `specs/120fps.md` section 8 to point here.)
5. Document the manual PresentMon protocol (capture command, columns, pass thresholds) in `docs/` or this spec's appendix so any future "it feels janky" report has a 5-minute ground-truth procedure.

Exit criteria: revert any single Phase 2-4 fix locally and at least one suite/bench gate fails.

## 5. Commands

```bash
# Build and run
cargo run --release                    # release build, required for any perf measurement
# Toggle FPS overlay in-app: Ctrl+Shift+F

# Tests
cargo test                             # all unit/integration tests
cargo xtask desktop-regression --list  # list suites
cargo xtask desktop-regression --suite settings-scroll-smoothness
cargo xtask desktop-regression --suite fps-overlay-scroll-updates

# Bench harness (report-only today; gains thresholds in Phase 5)
cargo run --release -- --bench

# Quality
cargo clippy
cargo fmt --check
```

```powershell
# External ground truth (Phase 0 baseline and Phase 4 acceptance)
# https://github.com/GameTechDev/PresentMon
.\PresentMon.exe --process_name terminal-manager.exe --output_file scroll.csv
# Inspect MsBetweenPresents, MsBetweenDisplayChange, PresentMode, GPUBusy
# Pass: MsBetweenDisplayChange p99 <= 12.5ms at 120Hz, < 2% rows with DisplayedTime=NA

# Environment facts referenced by Phase 0
Get-CimInstance Win32_VideoController | Select Name, CurrentRefreshRate, MaxRefreshRate
# 2026-06-11 reading: AMD Radeon 890M, CurrentRefreshRate 120, MaxRefreshRate 500
```

## 6. Project Structure

| Path | Role | Expected change |
|---|---|---|
| `src/ui/fps_overlay.rs` | FPS overlay | Phase 1: interval-based fps, new rows, throttled rebuilds, fixed test |
| `crates/unshit-framework/crates/unshit-app/src/app.rs` | Frame loop, smooth scroll, wheel normalization, metrics | Phases 1-4: heaviest churn; ends with one animation-tick path |
| `crates/unshit-framework/crates/unshit-app/src/frame_pacer.rs` | Pacer | Phase 4: true-period intervals, demote to work floor; delete 120Hz clamp |
| `crates/unshit-framework/crates/unshit-app/src/frame_probe.rs` | [FRAME] quantiles | Phase 1: interval channel |
| `crates/unshit-framework/crates/unshit-renderer/src/gpu.rs` | Present mode, surface config | Phase 4: Fifo or compositor clock; Phase 1 (optional): timestamp queries |
| `crates/unshit-framework/crates/unshit-renderer/src/batch.rs` | Fractional scroll offsets | Phase 3: terminal grid sub-row translation |
| `src/ui/terminal_grid.rs` | Wheel handler, grid build | Phase 2: accumulator; Phase 3: pixel-space animated scroll |
| `src/terminal/mod.rs` | Scrollback offset | Phase 3: sub-row position, output anchoring |
| `src/bench.rs` | Bench harness | Phase 1: interval metrics; Phase 5: thresholds |
| `src/diagnostics/snapshot.rs` | Diagnostics JSON | Phase 1: frame timestamp ring |
| `xtask/src/desktop_regression/suites/settings_scroll.rs` | Scroll suites | Phase 5: distribution asserts, new terminal-scroll suite |
| `tests/fixtures/perf_baseline.json` (new) | Perf baseline | Phase 5 |
| `specs/120fps.md` | Predecessor spec | Phase 5: amend metric definitions and testing strategy to point here |

Framework-first policy applies: pacing, present mode, smooth scroll, and metrics plumbing live in `crates/unshit-framework`; only terminal-specific scroll semantics live in app code.

## 7. Code Style

Inherits `specs/120fps.md` section 7 verbatim (TDD mandatory, no comments unless non-obvious why, no backward-compat shims, atomic conventional commits without AI co-authors, no mocked-PTY perf tests, `/simplify` after each green phase, no em dashes). Additions:

1. Perf fixes in Phases 2-4 each need a failing-then-passing check at the level the defect lives: unit test for input math, suite assert for cadence, PresentMon table for presentation.
2. Changelog fragment in `changelog/unreleased/` for every `feat:`/`fix:` commit, per repo convention.

## 8. Testing Strategy

### Unit tests

* Interval fps: timestamps 8.33ms apart with arbitrary `total_us` yield ~120 fps; bursty timestamps yield the windowed count, never 1/work.
* Wheel accumulator: property test, sum of applied rows over any delta sequence equals `trunc(sum(deltas)/cell_h)` with |carry| < 1 row; no event maps to more rows than its own delta.
* LineDelta normalization: 1 notch delivered as 1x3.0 or 5x0.6 scrolls the same distance.
* SmoothScroll retarget: mid-flight notch extends the target without velocity discontinuity (exists for settings path; reuse for terminal).
* Pacer: intervals derive from true period at 120/144/240/500Hz; the 8ms clamp is gone (rewrite the `with_refresh_rate_mhz_120000_uses_timer_compensated_8ms` test).
* Animation sampling: position function evaluated at predicted present time, not wake time (inject a fake clock).

### Integration / xtask suites

* Burst-scroll cadence: interval p99, stddev, dropped ratio per gate H2/H3 thresholds via the diagnostics timestamp ring.
* Terminal scroll: notch proportionality, >= 12 animation frames per notch, sub-row offsets nonzero mid-animation.
* Scroll during streaming PTY output: viewport anchored (gate S3).
* Overlay observer: enabling the overlay does not change `can_fast_paint_smooth_scroll` eligibility.

### External validation (manual, gating)

* PresentMon protocol from section 5 on the user's hardware at 120Hz, recorded into this spec per phase.
* Side-by-side feel comparison with VS Code (`editor.smoothScrolling` on) after Phase 3 and after Phase 4.

### Visual verification

Per repo convention, UI-affecting changes are verified with app-driven screenshots before commit, and the user confirms feel on real hardware:

* [ ] User scrolled a full scrollback at 120Hz and saw continuous eased motion, no row snapping, no hitches.
* [ ] User confirmed overlay fps reads <= 120 and stays honest while scrolling.
* [ ] User repeated at the panel's higher refresh mode after Phase 4 (S1).

## 9. Boundaries

### Always do

* Measure before and after every phase with the same PresentMon protocol.
* Keep work-time metrics intact while adding interval metrics; both matter, only the label was wrong.
* Land each phase as independently revertible commits.

### Ask first

* Before choosing Phase 4 Option B (compositor-clock thread) over Option A (Fifo): it adds a platform-specific thread and OS API surface.
* Before changing default scroll feel parameters (duration, easing) away from the settings-page tuning already matched to the Edge baseline.
* Before touching `desired_maximum_frame_latency` beyond 1-2.

### Never do

* Never present faster than the display refresh during scroll (waste + judder).
* Never reintroduce a wall-clock interval that undercuts the display period ("timer compensation").
* Never block the winit event-loop thread outside the OS wait primitive.
* Never gate a perf claim on the in-app overlay alone; PresentMon (or DXGI frame statistics) is the arbiter for presentation claims.

## 10. Open Questions (resolve in Phase 0 unless noted)

1. **Actual present mode on AMD Vulkan**: does the surface offer Mailbox at all, or does the app run Immediate today? Changes the urgency of Phase 4 and the interpretation of every baseline number.
2. **Why is the panel at 120Hz** when it supports 500Hz: cable/DSC limit, Parsec adapter, or deliberate? If raising it is possible, S1 verification needs it; gates stay defined at 120Hz regardless.
3. **Fifo vs compositor clock** (Phase 4 decision): Fifo is simpler but blocks in present/acquire, moving where latency lives; the Zed-style compositor clock keeps Mailbox latency but adds a vsync thread. Decide on Phase 0/Phase 4 latency data.
4. **Sub-row rendering cost** (Phase 3 step B): drawing rows+1 with a translation may interact with the glyph cache and damage tracking; validate the cost on the 890M before committing to per-frame fractional offsets.
5. **GPU timestamp queries**: wgpu `TIMESTAMP_QUERY` support/precision on this AMD driver; only needed if Phase 0 GPUBusy is material.
6. **Does `input-latency-histogram` graduate to default-on** once its end timestamp is corrected (currently event-to-submit, mislabeled)? Recommend yes for diagnostic builds.

## 11. Done Definition

The spec is complete when:

* All hard gates H1-H9 pass on the user's hardware at 120Hz, evidenced by a PresentMon table recorded in this spec.
* The canonical repro shows eased sub-row scrolling the user judges at parity with VS Code.
* The overlay, window title, `[FRAME]` lines, bench, and diagnostics all share one fps definition (presented cadence) plus separate work-time stats.
* The xtask suites fail when any single Phase 2-4 fix is reverted.
* `specs/120fps.md` is amended to reference this spec's metric definitions and the perf-baseline enforcement actually exists.
* CLAUDE.md gains the new invariants: fps means presentation cadence; never block the event loop; never pace below the display period; scroll input is accumulated fractionally and animated.

---

End of spec. Awaiting confirmation. Recommended start: Phase 0 (one evening of measurement, zero risk) in parallel with Phase 1 (honest metrics), since every later phase's acceptance depends on trustworthy numbers.
