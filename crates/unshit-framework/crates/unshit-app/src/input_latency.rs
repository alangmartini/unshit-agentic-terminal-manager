//! Input latency histogram instrument.
//!
//! Records, per rendered frame, the nanosecond delta from the first
//! winit input event that arrived this frame to the moment
//! `surface.present()` returns. Also tracks input events coalesced into
//! each frame and events that arrived mid draw (between frame start and
//! present).
//!
//! Modelled on Zed's `input_latency` instrument in
//! `zed-industries/zed` at `crates/gpui/src/window.rs` (the
//! `input_latency_snapshot` public API around line 1589, with the ring
//! of histograms defined at lines 1032..1103 in the source at the time
//! this module was written).
//!
//! Everything here is gated behind the `input-latency-histogram` cargo
//! feature. Release builds without the feature add zero code, zero
//! allocations, and zero runtime cost. The feature gate lives on the
//! `mod input_latency` declaration in `lib.rs`; duplicating it here
//! would trip the `duplicated_attributes` lint.

use hdrhistogram::Histogram;
use std::time::Instant;

/// Cloneable point in time snapshot of the histograms.
///
/// Returned by [`App::input_latency_snapshot`](crate::app::App::input_latency_snapshot)
/// and by the [`AppConfig::on_input_latency`](crate::app::AppConfig::on_input_latency)
/// per frame callback.
#[derive(Clone, Debug)]
pub struct InputLatencySnapshot {
    /// Histogram of first input to present latency in nanoseconds.
    pub latency_ns: Histogram<u64>,
    /// Histogram of input events coalesced into one rendered frame.
    pub events_per_frame: Histogram<u64>,
    /// Count of events that arrived after `mark_frame_start` and were
    /// therefore dropped from the current frame's latency accounting.
    pub mid_draw_events_dropped: u64,
    /// Total frames presented since tracker construction.
    pub frames_presented: u64,
    /// Total input events observed since tracker construction.
    pub events_observed: u64,
}

/// Tracks per frame input latency and coalescing statistics.
///
/// Lifecycle:
/// 1. Each winit input event calls [`record_event`].
/// 2. The top of `WindowEvent::RedrawRequested` calls
///    [`mark_frame_start`].
/// 3. After `surface.present()` returns (in practice: immediately after
///    the `on_frame_metrics` callback fires) the paint handler calls
///    [`record_frame_presented`].
pub struct InputLatencyTracker {
    first_event_at: Option<Instant>,
    events_this_frame: u64,
    frame_in_progress: bool,
    latency_ns: Histogram<u64>,
    events_per_frame: Histogram<u64>,
    mid_draw_events_dropped: u64,
    frames_presented: u64,
    events_observed: u64,
}

impl InputLatencyTracker {
    /// Construct a fresh tracker with empty histograms.
    ///
    /// Histograms are sized for three significant figures over the full
    /// `u64` range, which covers the nanosecond latencies (up to ~18s
    /// of headroom) and event counts (up to ~18 quintillion) we care
    /// about without any per record allocation.
    pub fn new() -> Result<Self, hdrhistogram::errors::CreationError> {
        Ok(Self {
            first_event_at: None,
            events_this_frame: 0,
            frame_in_progress: false,
            latency_ns: Histogram::new(3)?,
            events_per_frame: Histogram::new(3)?,
            mid_draw_events_dropped: 0,
            frames_presented: 0,
            events_observed: 0,
        })
    }

    /// Called from every winit input event handler.
    ///
    /// Uses the passed `Instant` because winit does not expose usable
    /// monotonic timestamps on all platforms. Call sites stamp
    /// `Instant::now()` at the top of `window_event` before any other
    /// work executes.
    ///
    /// Events that arrive while a frame is in progress (between
    /// [`mark_frame_start`] and [`record_frame_presented`]) are counted
    /// as mid draw drops and do not contribute to the current frame's
    /// latency sample. The NEXT frame will pick them up via its own
    /// `first_event_at`.
    pub fn record_event(&mut self, now: Instant) {
        self.events_observed = self.events_observed.saturating_add(1);
        if self.frame_in_progress {
            self.mid_draw_events_dropped = self.mid_draw_events_dropped.saturating_add(1);
            return;
        }
        if self.first_event_at.is_none() {
            self.first_event_at = Some(now);
        }
        self.events_this_frame = self.events_this_frame.saturating_add(1);
    }

    /// Called from the top of `WindowEvent::RedrawRequested`, before
    /// any tree build, layout, or render work and before the frame
    /// pacer early return.
    ///
    /// Putting this before the pacer check is deliberate: pacer skipped
    /// frames still flip the `frame_in_progress` flag so that events
    /// arriving during the sleep period are NOT folded into the next
    /// frame's latency sample. That would double count them.
    pub fn mark_frame_start(&mut self) {
        self.frame_in_progress = true;
    }

    /// Called after `surface.present()` returns and the paint handler
    /// has already fired `on_frame_metrics`.
    ///
    /// If any events arrived this frame the delta from the first event
    /// to `now` is recorded into [`latency_ns`]. Frames with zero
    /// events do not write to the latency histogram (we cannot measure
    /// latency without an input). Frames with zero events still
    /// increment [`frames_presented`].
    pub fn record_frame_presented(&mut self, now: Instant) {
        if let Some(t0) = self.first_event_at.take() {
            let delta_ns = now.saturating_duration_since(t0).as_nanos();
            let clamped = delta_ns.min(u64::MAX as u128) as u64;
            let _ = self.latency_ns.record(clamped);
        }
        if self.events_this_frame > 0 {
            let _ = self.events_per_frame.record(self.events_this_frame);
        }
        self.events_this_frame = 0;
        self.frame_in_progress = false;
        self.frames_presented = self.frames_presented.saturating_add(1);
    }

    /// Return a point in time clone of the histograms and counters.
    ///
    /// `Histogram<u64>::clone` is not free; callers should snapshot
    /// once per frame at most, never once per event.
    pub fn snapshot(&self) -> InputLatencySnapshot {
        InputLatencySnapshot {
            latency_ns: self.latency_ns.clone(),
            events_per_frame: self.events_per_frame.clone(),
            mid_draw_events_dropped: self.mid_draw_events_dropped,
            frames_presented: self.frames_presented,
            events_observed: self.events_observed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn fresh_tracker() -> InputLatencyTracker {
        InputLatencyTracker::new().expect("histogram construction cannot fail")
    }

    #[test]
    fn records_zero_events_do_not_write_latency_histogram() {
        let mut t = fresh_tracker();
        t.mark_frame_start();
        t.record_frame_presented(Instant::now());
        let snap = t.snapshot();
        assert_eq!(snap.latency_ns.len(), 0);
        assert_eq!(snap.events_per_frame.len(), 0);
        assert_eq!(snap.frames_presented, 1);
        assert_eq!(snap.events_observed, 0);
    }

    #[test]
    fn first_event_timestamp_wins_on_multiple_events() {
        let mut t = fresh_tracker();
        let t0 = Instant::now();
        let t1 = t0 + Duration::from_millis(2);
        let t2 = t0 + Duration::from_millis(5);
        t.record_event(t0);
        t.record_event(t1);
        t.record_event(t2);
        let present = t0 + Duration::from_millis(10);
        t.record_frame_presented(present);
        let snap = t.snapshot();
        assert_eq!(snap.latency_ns.len(), 1);
        let recorded = snap.latency_ns.value_at_quantile(1.0);
        let expected_ns = Duration::from_millis(10).as_nanos() as u64;
        let expected_low = expected_ns.saturating_sub(expected_ns / 100);
        let expected_high = expected_ns + expected_ns / 100;
        assert!(
            recorded >= expected_low && recorded <= expected_high,
            "recorded {} ns not within 1% of expected {} ns",
            recorded,
            expected_ns
        );
    }

    #[test]
    fn events_per_frame_counts_exact() {
        let mut t = fresh_tracker();
        let now = Instant::now();
        for i in 0..7 {
            t.record_event(now + Duration::from_micros(i));
        }
        t.record_frame_presented(now + Duration::from_millis(5));
        let snap = t.snapshot();
        assert_eq!(snap.events_per_frame.value_at_quantile(1.0), 7);
        assert_eq!(snap.events_observed, 7);
    }

    #[test]
    fn mid_draw_event_counts_do_not_pollute_latency() {
        let mut t = fresh_tracker();
        let now = Instant::now();
        t.mark_frame_start();
        t.record_event(now);
        t.record_event(now + Duration::from_micros(10));
        // No first_event_at set; no events_this_frame; frame present
        // records nothing to latency.
        t.record_frame_presented(now + Duration::from_millis(1));
        let snap = t.snapshot();
        assert_eq!(snap.mid_draw_events_dropped, 2);
        assert_eq!(snap.latency_ns.len(), 0);
        assert_eq!(snap.events_per_frame.len(), 0);
        assert_eq!(snap.events_observed, 2);
    }

    #[test]
    fn reset_behaviour_frame_to_frame() {
        let mut t = fresh_tracker();
        let base = Instant::now();

        // Frame A: 2 events.
        t.record_event(base);
        t.record_event(base + Duration::from_micros(100));
        t.record_frame_presented(base + Duration::from_millis(5));

        // Frame B: 4 events.
        let b_start = base + Duration::from_millis(10);
        for i in 0..4 {
            t.record_event(b_start + Duration::from_micros(i));
        }
        t.record_frame_presented(b_start + Duration::from_millis(3));

        let snap = t.snapshot();
        assert_eq!(snap.frames_presented, 2);
        assert_eq!(snap.events_observed, 6);
        // Events per frame histogram holds BOTH samples (2 and 4).
        assert_eq!(snap.events_per_frame.len(), 2);
        assert_eq!(snap.events_per_frame.value_at_quantile(0.0), 2);
        assert_eq!(snap.events_per_frame.value_at_quantile(1.0), 4);
    }

    #[test]
    fn snapshot_is_independent_clone() {
        let mut t = fresh_tracker();
        let now = Instant::now();
        t.record_event(now);
        t.record_frame_presented(now + Duration::from_millis(1));
        let snap1 = t.snapshot();
        let frames_first = snap1.frames_presented;
        let latency_len_first = snap1.latency_ns.len();

        // More activity: snap1 must remain unchanged.
        t.record_event(now + Duration::from_millis(2));
        t.record_frame_presented(now + Duration::from_millis(3));
        let snap2 = t.snapshot();
        assert_eq!(snap1.frames_presented, frames_first);
        assert_eq!(snap1.latency_ns.len(), latency_len_first);
        assert_eq!(snap2.frames_presented, 2);
        assert_eq!(snap2.latency_ns.len(), 2);
    }

    #[test]
    fn record_frame_presented_increments_frames_counter() {
        let mut t = fresh_tracker();
        let now = Instant::now();
        for _ in 0..5 {
            t.record_frame_presented(now);
        }
        assert_eq!(t.snapshot().frames_presented, 5);
    }

    #[test]
    fn saturating_counters_do_not_panic_at_boundary() {
        let mut t = fresh_tracker();
        // Force the counter to u64::MAX - 1, then record twice.
        t.events_observed = u64::MAX - 1;
        t.record_event(Instant::now());
        t.record_event(Instant::now());
        assert_eq!(t.snapshot().events_observed, u64::MAX);
    }
}
