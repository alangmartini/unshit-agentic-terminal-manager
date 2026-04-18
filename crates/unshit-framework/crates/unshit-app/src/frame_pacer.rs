//! Frame pacing utilities.
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

/// Snapshot of dirty signals fed into [`FramePacer::should_schedule_next_paint`].
///
/// Collected right after a paint finishes so the event loop can decide
/// whether the next vsync needs to produce another frame. Any field set to
/// `true` indicates that work remains and the pacer must schedule a follow-up
/// paint.
///
/// The struct is pure data so the decision can be unit-tested without
/// dragging in winit. All fields default to `false`, i.e. "everything clean,
/// nothing pending".
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DirtySignals {
    /// `state.needs_rebuild` snapshot.
    pub needs_rebuild: bool,
    /// `state.needs_restyle` snapshot.
    pub needs_restyle: bool,
    /// `state.needs_relayout` snapshot.
    pub needs_relayout: bool,
    /// External event channel still has pending items. PTY chunks that
    /// arrived during the paint land here and would otherwise wait until the
    /// next external wake signal.
    pub has_pending_events: bool,
    /// At least one node in the arena still carries PAINT or SUBTREE_PAINT
    /// dirty after the clear pass. Normally always `false` once
    /// `clear_paint_flags_subtree` ran; serves as a safety net when a
    /// subsystem sets paint dirty after the clear call.
    pub any_node_paint_dirty: bool,
}

impl DirtySignals {
    /// True when any dirty signal is set, i.e. the event loop should paint
    /// again on the next vsync rather than going fully idle.
    pub fn is_dirty(self) -> bool {
        self.needs_rebuild
            || self.needs_restyle
            || self.needs_relayout
            || self.has_pending_events
            || self.any_node_paint_dirty
    }
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

    /// Decide whether the event loop should paint another frame after the
    /// current paint finishes.
    ///
    /// This is the "vsync-locked paint loop" hook: when any dirty signal is
    /// set we return a [`PaceDecision`] so the caller can either
    /// `request_redraw` immediately or park the event loop with
    /// `ControlFlow::WaitUntil(deadline)`. When nothing is dirty we return
    /// `None` and the event loop falls back to reactive `ControlFlow::Wait`.
    ///
    /// Keeping this a pure function lets unit tests drive the decision
    /// without instantiating winit.
    pub fn should_schedule_next_paint(
        &self,
        now: Instant,
        dirty: DirtySignals,
    ) -> Option<PaceDecision> {
        if !dirty.is_dirty() {
            return None;
        }
        Some(self.on_redraw_requested(now))
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

    #[test]
    fn dirty_signals_default_is_all_clean() {
        let s = DirtySignals::default();
        assert!(!s.is_dirty());
        assert!(!s.needs_rebuild);
        assert!(!s.needs_restyle);
        assert!(!s.needs_relayout);
        assert!(!s.has_pending_events);
        assert!(!s.any_node_paint_dirty);
    }

    #[test]
    fn dirty_signals_any_bit_triggers_is_dirty() {
        // Each flag in isolation must report dirty.
        for s in [
            DirtySignals { needs_rebuild: true, ..Default::default() },
            DirtySignals { needs_restyle: true, ..Default::default() },
            DirtySignals { needs_relayout: true, ..Default::default() },
            DirtySignals { has_pending_events: true, ..Default::default() },
            DirtySignals { any_node_paint_dirty: true, ..Default::default() },
        ] {
            assert!(s.is_dirty(), "{:?} should be dirty", s);
        }
    }

    #[test]
    fn should_schedule_next_paint_returns_none_when_clean() {
        // Success criterion: idle stays idle. Helper must return None so the
        // event loop parks on ControlFlow::Wait rather than spinning.
        let mut pacer = FramePacer::with_min_interval(Duration::from_millis(8));
        let t0 = Instant::now();
        pacer.record_paint(t0);

        let decision = pacer.should_schedule_next_paint(t0, DirtySignals::default());
        assert_eq!(decision, None);
    }

    #[test]
    fn should_schedule_next_paint_when_needs_rebuild_set() {
        // Rebuild pending after a paint means new external work (e.g. PTY
        // chunk queued during paint). Helper must return Some(_) so the
        // event loop queues another frame.
        let mut pacer = FramePacer::with_min_interval(Duration::from_millis(8));
        let t0 = Instant::now();
        pacer.record_paint(t0);

        // Ask at t0 + 2ms: within the pacer interval, so WaitUntil.
        let now = t0 + Duration::from_millis(2);
        let dirty = DirtySignals { needs_rebuild: true, ..Default::default() };
        let decision = pacer.should_schedule_next_paint(now, dirty);
        assert_eq!(decision, Some(PaceDecision::WaitUntil(t0 + Duration::from_millis(8))));
    }

    #[test]
    fn should_schedule_next_paint_when_node_paint_dirty_but_flags_clear() {
        // Regression coverage: needs_rebuild == false but a node still has
        // PAINT dirty (edge case where a subsystem re-dirties after clear).
        // Helper must still return Some(_) so the node gets repainted.
        let mut pacer = FramePacer::with_min_interval(Duration::from_millis(8));
        let t0 = Instant::now();
        pacer.record_paint(t0);

        let dirty = DirtySignals {
            needs_rebuild: false,
            needs_restyle: false,
            needs_relayout: false,
            has_pending_events: false,
            any_node_paint_dirty: true,
        };
        let decision = pacer.should_schedule_next_paint(t0 + Duration::from_millis(2), dirty);
        assert!(matches!(decision, Some(PaceDecision::WaitUntil(_))));
    }

    #[test]
    fn should_schedule_next_paint_after_interval_returns_paint_now() {
        // Dirty and interval already elapsed: paint immediately rather than
        // waiting an extra 8ms.
        let mut pacer = FramePacer::with_min_interval(Duration::from_millis(8));
        let t0 = Instant::now();
        pacer.record_paint(t0);

        let now = t0 + Duration::from_millis(12);
        let dirty = DirtySignals { has_pending_events: true, ..Default::default() };
        let decision = pacer.should_schedule_next_paint(now, dirty);
        assert_eq!(decision, Some(PaceDecision::PaintNow));
    }

    #[test]
    fn should_schedule_next_paint_first_paint_is_immediate_when_dirty() {
        // With no prior paint recorded, a dirty follow-up paints immediately.
        let pacer = FramePacer::new();
        let dirty = DirtySignals { needs_relayout: true, ..Default::default() };
        let decision = pacer.should_schedule_next_paint(Instant::now(), dirty);
        assert_eq!(decision, Some(PaceDecision::PaintNow));
    }
}
