//! Frame pacing utilities (Tier 1, task 3).
//!
//! Coalesces redraw requests so that the event loop paints at most once per
//! `min_frame_interval`. The framework currently does not expose a vsync
//! callback from winit, so we emulate it with a coalescing timer. When the
//! framework learns about a real vsync hook we can swap in that signal
//! without touching the call sites.
//!
//! The design purposely keeps the decision logic pure so it can be tested
//! deterministically with synthetic clocks. The frame loop owns a
//! [`FramePacer`] and asks it before each paint whether it is OK to paint
//! now; when the answer is "wait", the pacer returns the absolute wake time
//! the caller should hand to winit's `ControlFlow::WaitUntil`.
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

/// Coalescing frame pacer. Default interval is 8ms, matching Ghostty's
/// default and close to Kitty's 10ms input batch interval. The interval is
/// a ceiling on paint rate; actual paints may be rarer when there is
/// nothing to redraw.
pub struct FramePacer {
    /// Minimum interval between two consecutive paints.
    min_interval: Duration,
    /// Timestamp of the last paint this pacer observed.
    last_paint: Option<Instant>,
}

impl Default for FramePacer {
    fn default() -> Self {
        Self::new()
    }
}

impl FramePacer {
    /// 8 milliseconds. See module documentation for the rationale.
    pub const DEFAULT_MIN_INTERVAL: Duration = Duration::from_millis(8);

    /// Construct a pacer with the default coalescing interval (8ms).
    pub fn new() -> Self {
        Self { min_interval: Self::DEFAULT_MIN_INTERVAL, last_paint: None }
    }

    /// Construct a pacer with a custom coalescing interval. Useful for tests
    /// and future experimentation.
    pub fn with_min_interval(min_interval: Duration) -> Self {
        Self { min_interval, last_paint: None }
    }

    /// Minimum interval between two paints. Exposed for diagnostics.
    pub fn min_interval(&self) -> Duration {
        self.min_interval
    }

    /// Timestamp of the last paint recorded.
    pub fn last_paint(&self) -> Option<Instant> {
        self.last_paint
    }

    /// Record that a paint occurred at `now`. Subsequent redraw requests
    /// are gated against this timestamp.
    pub fn record_paint(&mut self, now: Instant) {
        self.last_paint = Some(now);
    }

    /// Decide whether the caller should paint at `now`. Returns
    /// [`PaceDecision::PaintNow`] when the interval has elapsed (or no paint
    /// has occurred yet) and [`PaceDecision::WaitUntil`] otherwise.
    ///
    /// The first paint after construction always proceeds immediately.
    pub fn on_redraw_requested(&self, now: Instant) -> PaceDecision {
        match self.last_paint {
            None => PaceDecision::PaintNow,
            Some(last) => {
                let elapsed = now.saturating_duration_since(last);
                if elapsed >= self.min_interval {
                    PaceDecision::PaintNow
                } else {
                    PaceDecision::WaitUntil(last + self.min_interval)
                }
            }
        }
    }
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
}
