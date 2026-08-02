//! Frame pacing utilities.
//!
//! The pacer no longer emulates vsync. On a vsync-paced surface (the
//! Fifo-family present modes, the default since Phase 4 of
//! `specs/scroll-smoothness-and-honest-fps.md`) the renderer's blocking
//! swapchain acquire anchors the paint loop to the display clock, and this
//! module survives only as the metrics floor: `min_interval` is reported as
//! `FrameMetrics::pacer_min_interval_ns` and `record_paint` keeps the
//! bookkeeping consumers expect. On surfaces without a blocking present
//! mode (the Timer fallback) the pacer still coalesces redraw requests so
//! the event loop paints at most once per `min_interval`, which is now the
//! display's true period rather than a wall-clock interval that undercuts
//! it.
//!
//! The design purposely keeps the decision logic pure so it can be tested
//! deterministically with synthetic clocks. In Timer mode the frame loop
//! asks the pacer before each paint whether it is OK to paint now; when the
//! answer is "wait", the pacer returns the absolute wake time the caller
//! should hand to winit's `ControlFlow::WaitUntil`.
//!
//! This module never renders. It only schedules.

use std::time::{Duration, Instant};

/// Decision produced by [`FramePacer::on_redraw_requested`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaceDecision {
    /// Paint immediately. Caller should mark `last_paint` on completion via
    /// [`FramePacer::record_paint`].
    PaintNow,
    /// Skip the paint this time and reschedule with the contained wake time.
    /// The caller should hand this to winit `WaitUntil` and also call
    /// `request_redraw()` so the redraw fires as soon as the timer expires.
    WaitUntil(Instant),
}

/// Coalescing frame pacer. The interval is the active monitor's true
/// refresh period, derived via [`Self::with_refresh_rate_mhz`]; the legacy
/// 8ms default (matching Ghostty's default and close to Kitty's 10ms input
/// batch interval) is kept only as the fallback when the platform cannot
/// report a believable refresh rate. The interval is a ceiling on paint
/// rate; actual paints may be rarer when there is nothing to redraw, and
/// on vsync-paced surfaces the blocking acquire (not this pacer) is what
/// enforces the cadence.
pub struct FramePacer {
    /// Minimum interval between two consecutive paints.
    min_interval: Duration,
    /// Timestamp of the last paint this pacer observed.
    last_paint: Option<Instant>,
    /// Absolute phase boundary at which the next timer-paced frame should
    /// be presented. Unlike `last_paint + min_interval`, this remains
    /// anchored when the OS wakes a little late, so sub-millisecond wake
    /// jitter cannot accumulate into lost refresh slots.
    next_presentation_target: Option<Instant>,
    /// How far before `next_presentation_target` rendering may begin. Zero
    /// preserves the traditional start-on-phase behavior; a non-zero lead
    /// lets the renderer absorb variable work and hold the completed frame
    /// until the stable presentation phase.
    presentation_lead: Duration,
}

impl Default for FramePacer {
    fn default() -> Self {
        Self::new()
    }
}

impl FramePacer {
    /// 8 milliseconds. See module documentation for the rationale. Used as
    /// the fallback when the active monitor does not report a refresh rate.
    pub const DEFAULT_MIN_INTERVAL: Duration = Duration::from_millis(8);

    /// Floor on the interval derived from a refresh rate. Sub-millisecond
    /// periods (e.g. a hypothetical 1000Hz panel) would cause the event
    /// loop to wake so often that the pacer becomes useless; clamp to 1ms.
    pub const MIN_DERIVED_INTERVAL: Duration = Duration::from_millis(1);

    /// Floor on believable refresh-rate reports. Anything below 10Hz is
    /// driver garbage, not a panel; treat it like "rate unknown" instead
    /// of pacing the app at seconds-long intervals.
    const MIN_BELIEVABLE_MHZ: u32 = 10_000;

    /// Construct a pacer with the default coalescing interval (8ms).
    pub fn new() -> Self {
        Self {
            min_interval: Self::DEFAULT_MIN_INTERVAL,
            last_paint: None,
            next_presentation_target: None,
            presentation_lead: Duration::ZERO,
        }
    }

    /// Construct a pacer with a custom coalescing interval. Useful for tests
    /// and future experimentation.
    pub fn with_min_interval(min_interval: Duration) -> Self {
        Self {
            min_interval,
            last_paint: None,
            next_presentation_target: None,
            presentation_lead: Duration::ZERO,
        }
    }

    /// Construct a pacer whose coalescing interval is derived from the
    /// active monitor's refresh rate, measured in millihertz. Pass 0 to
    /// force the historic 8ms fallback. Values above ~1000Hz are clamped
    /// to [`Self::MIN_DERIVED_INTERVAL`].
    pub fn with_refresh_rate_mhz(mhz: u32) -> Self {
        Self {
            min_interval: Self::interval_from_mhz(mhz),
            last_paint: None,
            next_presentation_target: None,
            presentation_lead: Duration::ZERO,
        }
    }

    /// Update the coalescing interval after construction. Called when the
    /// window crosses a monitor boundary and the new display reports a
    /// different refresh rate. Pass 0 to fall back to the default.
    pub fn set_refresh_rate_mhz(&mut self, mhz: u32) {
        self.min_interval = Self::interval_from_mhz(mhz);
        self.presentation_lead = self.presentation_lead.min(self.min_interval);
        self.next_presentation_target =
            self.last_paint.and_then(|last| last.checked_add(self.min_interval));
    }

    /// Pure helper: `mhz -> Duration`, the display's true refresh period.
    /// `mhz == 0` (rate unknown) and sub-10Hz garbage reports return
    /// [`Self::DEFAULT_MIN_INTERVAL`] so the pacer keeps working when the
    /// compositor cannot enumerate a believable rate. Results are clamped
    /// to [`Self::MIN_DERIVED_INTERVAL`] on the fast end. The interval is
    /// never shortened below the true period (no "timer compensation"):
    /// a producer that undercuts the display period beats against the
    /// scanout and drops or doubles frames, the exact defect Phase 4 of
    /// the scroll-smoothness spec removed.
    pub fn interval_from_mhz(mhz: u32) -> Duration {
        if mhz < Self::MIN_BELIEVABLE_MHZ {
            return Self::DEFAULT_MIN_INTERVAL;
        }
        // mhz is frames per 1000 seconds. interval_ns = 1e12 / mhz.
        // Use u64 for the division: 1e12 does not fit in u32 for low rates.
        let interval_ns = 1_000_000_000_000u64 / (mhz as u64);
        Duration::from_nanos(interval_ns).max(Self::MIN_DERIVED_INTERVAL)
    }

    /// Minimum interval between two paints. Exposed for diagnostics.
    pub fn min_interval(&self) -> Duration {
        self.min_interval
    }

    /// Timestamp of the last paint recorded.
    pub fn last_paint(&self) -> Option<Instant> {
        self.last_paint
    }

    /// Allow timer-paced rendering to begin this far before the stable
    /// presentation phase. Values longer than one refresh period are clamped
    /// so a frame can never be prepared more than one slot ahead.
    pub fn set_presentation_lead(&mut self, lead: Duration) {
        self.presentation_lead = lead.min(self.min_interval);
    }

    pub fn presentation_lead(&self) -> Duration {
        self.presentation_lead
    }

    /// Presentation phase assigned to the frame that is currently due.
    /// `None` only before the first paint establishes an anchor.
    pub fn presentation_target(&self) -> Option<Instant> {
        self.next_presentation_target
    }

    fn render_deadline(&self) -> Option<Instant> {
        self.next_presentation_target
            .map(|target| target.checked_sub(self.presentation_lead).unwrap_or(target))
    }

    /// Record that a paint occurred at `now`. Subsequent redraw requests
    /// are gated against this timestamp.
    pub fn record_paint(&mut self, now: Instant) {
        self.last_paint = Some(now);
        self.next_presentation_target = Some(match self.next_presentation_target {
            None => now.checked_add(self.min_interval).unwrap_or(now),
            // A frame that began on or before its target consumes exactly
            // that phase, even when a presentation lead made the start early.
            Some(target) if now <= target => target.checked_add(self.min_interval).unwrap_or(now),
            // A late frame resumes only at a target that still leaves the
            // configured render lead available; otherwise it would be born
            // late again and could spin through stale deadlines.
            Some(target) => next_phase_deadline(
                target,
                now.checked_add(self.presentation_lead).unwrap_or(now),
                self.min_interval,
            ),
        });
    }

    /// Decide whether the caller should paint at `now`. Returns
    /// [`PaceDecision::PaintNow`] when the interval has elapsed (or no paint
    /// has occurred yet) and [`PaceDecision::WaitUntil`] otherwise.
    ///
    /// The first paint after construction always proceeds immediately.
    pub fn on_redraw_requested(&self, now: Instant) -> PaceDecision {
        match self.render_deadline() {
            None => PaceDecision::PaintNow,
            Some(deadline) if now >= deadline => PaceDecision::PaintNow,
            Some(deadline) => PaceDecision::WaitUntil(deadline),
        }
    }

    /// Absolute time at which the next speculative paint should fire.
    ///
    /// Returns `max(now, last_paint + min_interval)`. A speculative paint is
    /// one the event loop schedules during a "recently active" window even
    /// without a dirty flag, so that a PTY chunk or keystroke landing inside
    /// the frame interval is displayed on the very next vsync rather than
    /// waiting for the next external event. The pacer's `min_interval`
    /// remains the hard cap on paint rate, so successive speculative frames
    /// never run faster than 1 / `min_interval`.
    ///
    /// Caller is expected to hand the returned instant to winit
    /// `ControlFlow::WaitUntil` and to also call `request_redraw()` so the
    /// redraw fires as soon as the deadline elapses.
    pub fn speculative_deadline(&self, now: Instant) -> Instant {
        match self.render_deadline() {
            None => now,
            Some(deadline) => deadline.max(now),
        }
    }
}

/// Advance an existing timer phase past `now`, skipping whole missed slots.
/// The arithmetic path is O(1), including after a long suspend; an overflow
/// falls back to a fresh one-period deadline instead of returning a stale
/// instant that would spin the event loop.
fn next_phase_deadline(deadline: Instant, now: Instant, period: Duration) -> Instant {
    if period.is_zero() {
        return now;
    }
    let elapsed = now.saturating_duration_since(deadline);
    let slots = elapsed.as_nanos() / period.as_nanos() + 1;
    let advanced = u32::try_from(slots)
        .ok()
        .and_then(|slots| period.checked_mul(slots))
        .and_then(|advance| deadline.checked_add(advance));
    advanced.filter(|next| *next > now).or_else(|| now.checked_add(period)).unwrap_or(now)
}

/// Abstract source of the active display's refresh rate.
///
/// Implementors return the millihertz value from whatever backing API is
/// appropriate: winit's `Window::current_monitor().current_video_mode()`,
/// a platform-specific vsync query, or a test fake. Returning `None`
/// means the source could not determine a rate and the pacer should fall
/// back to [`FramePacer::DEFAULT_MIN_INTERVAL`].
pub trait MonitorRefreshSource {
    /// The active monitor's refresh rate in millihertz, if known.
    fn current_refresh_mhz(&self) -> Option<u32>;
}

/// Update a pacer's coalescing interval from the given refresh source.
///
/// `None` from the source maps to 0 mHz, which
/// [`FramePacer::interval_from_mhz`] converts into
/// [`FramePacer::DEFAULT_MIN_INTERVAL`]. Callers run this from
/// `WindowEvent::Moved` and scale-factor-change branches; it is safe to
/// call from any event handler as long as the cost of the source query
/// is acceptable. See the `app.rs` integration for the debounce policy.
pub fn refresh_pacer_from_source(pacer: &mut FramePacer, source: &dyn MonitorRefreshSource) {
    let mhz = source.current_refresh_mhz().unwrap_or(0);
    pacer.set_refresh_rate_mhz(mhz);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_decisions_eq(actual: PaceDecision, expected: PaceDecision) {
        match (actual, expected) {
            (PaceDecision::PaintNow, PaceDecision::PaintNow) => {}
            (PaceDecision::WaitUntil(a), PaceDecision::WaitUntil(b)) => {
                assert_eq!(a, b, "WaitUntil timestamps differ: {:?} vs {:?}", a, b);
            }
            (a, b) => panic!("decision mismatch: {:?} vs {:?}", a, b),
        }
    }

    #[test]
    fn first_paint_proceeds_immediately() {
        let pacer = FramePacer::new();
        let now = Instant::now();
        assert_decisions_eq(pacer.on_redraw_requested(now), PaceDecision::PaintNow);
    }

    #[test]
    fn second_paint_within_interval_waits() {
        let mut pacer = FramePacer::with_min_interval(Duration::from_millis(8));
        let t0 = Instant::now();
        pacer.record_paint(t0);

        let t1 = t0 + Duration::from_millis(2);
        assert_decisions_eq(
            pacer.on_redraw_requested(t1),
            PaceDecision::WaitUntil(t0 + Duration::from_millis(8)),
        );
    }

    #[test]
    fn paint_after_interval_proceeds() {
        let mut pacer = FramePacer::with_min_interval(Duration::from_millis(8));
        let t0 = Instant::now();
        pacer.record_paint(t0);

        let t1 = t0 + Duration::from_millis(9);
        assert_decisions_eq(pacer.on_redraw_requested(t1), PaceDecision::PaintNow);
    }

    #[test]
    fn exactly_at_interval_boundary_proceeds() {
        let mut pacer = FramePacer::with_min_interval(Duration::from_millis(8));
        let t0 = Instant::now();
        pacer.record_paint(t0);

        let t1 = t0 + Duration::from_millis(8);
        assert_decisions_eq(pacer.on_redraw_requested(t1), PaceDecision::PaintNow);
    }

    #[test]
    fn multiple_rapid_requests_all_wait_for_same_deadline() {
        // Twelve PTY chunks arriving in 1ms each must coalesce into a
        // single paint at the next 8ms boundary.
        let mut pacer = FramePacer::with_min_interval(Duration::from_millis(8));
        let t0 = Instant::now();
        pacer.record_paint(t0);
        let expected = t0 + Duration::from_millis(8);

        for i in 1..=12 {
            let t = t0 + Duration::from_micros(i * 500); // 0.5ms each = 6ms total
            assert_decisions_eq(pacer.on_redraw_requested(t), PaceDecision::WaitUntil(expected));
        }
    }

    #[test]
    fn late_wake_does_not_shift_the_refresh_phase() {
        let mut pacer = FramePacer::with_min_interval(Duration::from_millis(8));
        let t0 = Instant::now();
        pacer.record_paint(t0);

        // The OS wakes 500us after the first 8ms deadline. The next frame
        // still belongs at the 16ms phase boundary, not at 16.5ms.
        pacer.record_paint(t0 + Duration::from_micros(8_500));
        assert_decisions_eq(
            pacer.on_redraw_requested(t0 + Duration::from_millis(15)),
            PaceDecision::WaitUntil(t0 + Duration::from_millis(16)),
        );
    }

    #[test]
    fn repeated_wake_overshoot_does_not_accumulate() {
        let mut pacer = FramePacer::with_min_interval(Duration::from_millis(8));
        let t0 = Instant::now();
        pacer.record_paint(t0);

        // A 500us wake overshoot on every frame must remain a bounded
        // offset from the phase. Re-anchoring every deadline to the actual
        // wake would lose 60ms over these 120 frames (about seven 120Hz
        // presentation slots in one second).
        for frame in 1..=120u32 {
            let phase = Duration::from_millis(u64::from(frame) * 8);
            pacer.record_paint(t0 + phase + Duration::from_micros(500));
        }
        assert_decisions_eq(
            pacer.on_redraw_requested(t0 + Duration::from_millis(967)),
            PaceDecision::WaitUntil(t0 + Duration::from_millis(968)),
        );
    }

    #[test]
    fn badly_late_wake_skips_whole_missed_slots() {
        let mut pacer = FramePacer::with_min_interval(Duration::from_millis(8));
        let t0 = Instant::now();
        pacer.record_paint(t0);

        // A frame that arrives after the 8, 16, and 24ms boundaries must
        // resume on the 32ms boundary. Returning a stale deadline would
        // spin the event loop; re-anchoring at 26ms would add drift.
        pacer.record_paint(t0 + Duration::from_millis(26));
        assert_decisions_eq(
            pacer.on_redraw_requested(t0 + Duration::from_millis(27)),
            PaceDecision::WaitUntil(t0 + Duration::from_millis(32)),
        );
    }

    #[test]
    fn presentation_lead_wakes_before_the_phase_target() {
        let mut pacer = FramePacer::with_min_interval(Duration::from_millis(8));
        pacer.set_presentation_lead(Duration::from_millis(6));
        let t0 = Instant::now();
        pacer.record_paint(t0);

        assert_eq!(pacer.presentation_target(), Some(t0 + Duration::from_millis(8)));
        assert_decisions_eq(
            pacer.on_redraw_requested(t0 + Duration::from_millis(1)),
            PaceDecision::WaitUntil(t0 + Duration::from_millis(2)),
        );

        // The frame starts at 2ms and is held until its 8ms presentation
        // target. Recording that start consumes the target without
        // re-anchoring the phase.
        pacer.record_paint(t0 + Duration::from_millis(2));
        assert_eq!(pacer.presentation_target(), Some(t0 + Duration::from_millis(16)));
        assert_decisions_eq(
            pacer.on_redraw_requested(t0 + Duration::from_millis(9)),
            PaceDecision::WaitUntil(t0 + Duration::from_millis(10)),
        );
    }

    #[test]
    fn presentation_lead_is_clamped_to_one_period() {
        let mut pacer = FramePacer::with_min_interval(Duration::from_millis(8));
        pacer.set_presentation_lead(Duration::from_millis(20));
        assert_eq!(pacer.presentation_lead(), Duration::from_millis(8));
    }

    #[test]
    fn late_leaded_frame_skips_until_a_full_render_window_exists() {
        let mut pacer = FramePacer::with_min_interval(Duration::from_millis(8));
        pacer.set_presentation_lead(Duration::from_millis(6));
        let t0 = Instant::now();
        pacer.record_paint(t0);

        // Starting at 11ms has already missed the 8ms target. The 16ms
        // target is only 5ms away, less than the configured render lead,
        // so resume at the 24ms target / 18ms render deadline.
        pacer.record_paint(t0 + Duration::from_millis(11));
        assert_eq!(pacer.presentation_target(), Some(t0 + Duration::from_millis(24)));
        assert_decisions_eq(
            pacer.on_redraw_requested(t0 + Duration::from_millis(12)),
            PaceDecision::WaitUntil(t0 + Duration::from_millis(18)),
        );
    }

    #[test]
    fn custom_interval_is_respected() {
        let pacer = FramePacer::with_min_interval(Duration::from_millis(16));
        assert_eq!(pacer.min_interval(), Duration::from_millis(16));
    }

    #[test]
    fn default_min_interval_is_8ms() {
        assert_eq!(FramePacer::DEFAULT_MIN_INTERVAL, Duration::from_millis(8));
        assert_eq!(FramePacer::new().min_interval(), Duration::from_millis(8));
    }

    #[test]
    fn record_paint_updates_last_paint_timestamp() {
        let mut pacer = FramePacer::new();
        assert!(pacer.last_paint().is_none());
        let t = Instant::now();
        pacer.record_paint(t);
        assert_eq!(pacer.last_paint(), Some(t));
    }

    #[test]
    fn speculative_deadline_before_any_paint_returns_now() {
        // No paint has occurred yet, so a speculative frame can fire
        // immediately. The caller still has to hand the deadline to winit
        // WaitUntil, which treats a past deadline as "fire asap".
        let pacer = FramePacer::new();
        let now = Instant::now();
        assert_eq!(pacer.speculative_deadline(now), now);
    }

    #[test]
    fn speculative_deadline_respects_pacer_min_interval() {
        // Immediately after a paint, the next speculative frame must wait
        // until last_paint + min_interval to enforce the 125fps ceiling.
        let mut pacer = FramePacer::with_min_interval(Duration::from_millis(8));
        let t0 = Instant::now();
        pacer.record_paint(t0);

        let now = t0 + Duration::from_millis(2);
        assert_eq!(pacer.speculative_deadline(now), t0 + Duration::from_millis(8));
    }

    #[test]
    fn speculative_deadline_clamped_to_now_when_interval_elapsed() {
        // If the pacer interval has already elapsed (we were waiting longer
        // than min_interval for some reason), the deadline must not move
        // backward; fire at `now` instead.
        let mut pacer = FramePacer::with_min_interval(Duration::from_millis(8));
        let t0 = Instant::now();
        pacer.record_paint(t0);

        let now = t0 + Duration::from_millis(20);
        assert_eq!(pacer.speculative_deadline(now), now);
    }

    #[test]
    fn clock_skew_backwards_does_not_panic() {
        // saturating_duration_since guarantees no overflow when Instant
        // values appear to travel backwards (should never happen on winit
        // but paranoia is cheap).
        let mut pacer = FramePacer::with_min_interval(Duration::from_millis(8));
        let t0 = Instant::now();
        pacer.record_paint(t0 + Duration::from_millis(100));

        // Ask the pacer at a now that is before the recorded last_paint.
        let decision = pacer.on_redraw_requested(t0);
        // Elapsed saturates to zero, which is < min_interval, so we wait.
        let expected = t0 + Duration::from_millis(100) + Duration::from_millis(8);
        assert_decisions_eq(decision, PaceDecision::WaitUntil(expected));
    }

    // === Refresh-rate derived interval (capillary #82) ===

    #[test]
    fn with_refresh_rate_mhz_120000_gives_true_8333us_period() {
        // 120000 mHz == 120 Hz. The "timer compensated" 8ms clamp is gone:
        // an 8ms producer against an 8.333ms scanout drifts one frame
        // every ~25, the beat Phase 4 of the scroll-smoothness spec
        // measured and removed. The pacer now reports the true period.
        let pacer = FramePacer::with_refresh_rate_mhz(120_000);
        assert_eq!(pacer.min_interval(), Duration::from_nanos(8_333_333));
    }

    #[test]
    fn with_refresh_rate_mhz_144000_gives_6944us() {
        // 144000 mHz == 144 Hz. Expected interval: 1e12 / 144000 = 6_944_444 ns.
        let pacer = FramePacer::with_refresh_rate_mhz(144_000);
        assert_eq!(pacer.min_interval(), Duration::from_nanos(6_944_444));
    }

    #[test]
    fn with_refresh_rate_mhz_165000_gives_6060us() {
        // 165000 mHz == 165 Hz. Expected interval: 1e12 / 165000 = 6_060_606 ns.
        let pacer = FramePacer::with_refresh_rate_mhz(165_000);
        assert_eq!(pacer.min_interval(), Duration::from_nanos(6_060_606));
    }

    #[test]
    fn with_refresh_rate_mhz_240000_gives_4166us() {
        // 240000 mHz == 240 Hz. Expected interval: 1e12 / 240000 = 4_166_666 ns.
        let pacer = FramePacer::with_refresh_rate_mhz(240_000);
        assert_eq!(pacer.min_interval(), Duration::from_nanos(4_166_666));
    }

    #[test]
    fn with_refresh_rate_mhz_zero_falls_back_to_default() {
        // The monitor returned no refresh rate (None). Fallback must
        // equal the historic default so we never regress older behavior.
        let pacer = FramePacer::with_refresh_rate_mhz(0);
        assert_eq!(pacer.min_interval(), FramePacer::DEFAULT_MIN_INTERVAL);
        assert_eq!(pacer.min_interval(), FramePacer::new().min_interval());
    }

    #[test]
    fn with_refresh_rate_mhz_low_rates_pace_at_their_true_period() {
        // Pacing faster than a slow panel's period would only produce
        // frames the display can never show (the spec's "never undercut
        // the display period" boundary), so 24Hz and 60Hz panels get
        // their real periods rather than the old 8ms clamp.
        let pacer = FramePacer::with_refresh_rate_mhz(24_000);
        assert_eq!(pacer.min_interval(), Duration::from_nanos(41_666_666));

        let pacer = FramePacer::with_refresh_rate_mhz(60_000);
        assert_eq!(pacer.min_interval(), Duration::from_nanos(16_666_666));
    }

    #[test]
    fn with_refresh_rate_mhz_sub_10hz_garbage_falls_back_to_default() {
        // Sub-10Hz reports are driver garbage, not panels. Treat them
        // like "rate unknown" so the pacer never schedules seconds-long
        // intervals.
        let pacer = FramePacer::with_refresh_rate_mhz(9_999);
        assert_eq!(pacer.min_interval(), FramePacer::DEFAULT_MIN_INTERVAL);

        let pacer = FramePacer::with_refresh_rate_mhz(1);
        assert_eq!(pacer.min_interval(), FramePacer::DEFAULT_MIN_INTERVAL);
    }

    #[test]
    fn with_refresh_rate_mhz_absurd_1000000_clamps_to_1ms() {
        // 1000 Hz panels do not exist, but the pacer must never wake the
        // event loop at faster than MIN_DERIVED_INTERVAL regardless of
        // what the compositor reports.
        let pacer = FramePacer::with_refresh_rate_mhz(1_000_000);
        assert_eq!(pacer.min_interval(), FramePacer::MIN_DERIVED_INTERVAL);
        assert_eq!(pacer.min_interval(), Duration::from_millis(1));
    }

    #[test]
    fn set_refresh_rate_mhz_updates_existing_pacer() {
        let mut pacer = FramePacer::with_refresh_rate_mhz(120_000);
        assert_eq!(pacer.min_interval(), Duration::from_nanos(8_333_333));
        pacer.set_refresh_rate_mhz(144_000);
        assert_eq!(pacer.min_interval(), Duration::from_nanos(6_944_444));
    }

    #[test]
    fn interval_from_mhz_is_monotonic_decreasing_in_mhz() {
        // For any sorted increasing list of mhz values, the derived
        // interval must be non-increasing. Clamps at the ends are fine;
        // just no inversions between neighbors.
        let rates = [60_000u32, 75_000, 100_000, 120_000, 144_000, 165_000, 240_000, 360_000];
        let mut prev = FramePacer::interval_from_mhz(rates[0]);
        for r in &rates[1..] {
            let cur = FramePacer::interval_from_mhz(*r);
            assert!(
                cur <= prev,
                "interval at {}mhz ({:?}) must not exceed interval at slower rate ({:?})",
                r,
                cur,
                prev
            );
            prev = cur;
        }
    }

    // === Regression guards ===

    #[test]
    fn default_min_interval_is_still_8ms_after_refactor() {
        // regression for #81 item 1 fallback path: ensure the legacy
        // 8ms default never changes silently during a refactor.
        assert_eq!(FramePacer::new().min_interval(), Duration::from_millis(8));
        assert_eq!(FramePacer::DEFAULT_MIN_INTERVAL, Duration::from_millis(8));
    }

    #[test]
    fn speculative_deadline_respects_derived_interval_after_rate_change() {
        // regression for #82: when the window crosses monitors and the
        // pacer rate changes, the speculative deadline must use the new
        // shorter interval rather than a stale one.
        let mut pacer = FramePacer::with_refresh_rate_mhz(120_000);
        let t0 = Instant::now();
        pacer.record_paint(t0);

        // Before the rate change: speculative deadline is t0 + the true
        // 120Hz period.
        let expected_120 = t0 + Duration::from_nanos(8_333_333);
        assert_eq!(pacer.speculative_deadline(t0), expected_120);

        // Simulate moving to a 240Hz panel.
        pacer.set_refresh_rate_mhz(240_000);

        // After the rate change: speculative deadline is t0 + ~4.166ms.
        let expected_240 = t0 + Duration::from_nanos(4_166_666);
        assert_eq!(pacer.speculative_deadline(t0), expected_240);
    }
}
