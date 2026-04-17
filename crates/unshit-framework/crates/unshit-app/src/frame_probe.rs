//! Frame-time probe (Tier 1, task 4).
//!
//! Records per-frame wall-clock durations in a small ring buffer and emits
//! p50/p95/p99 quantiles plus min/max/frame count once per second. Debug
//! builds only: gated on `#[cfg(debug_assertions)]` at every call site so
//! release binaries pay zero runtime cost.
//!
//! The probe never calls log directly; callers feed frames in via
//! [`FrameProbe::record_frame`] and poll [`FrameProbe::maybe_emit`] at the
//! end of every frame. This shape keeps the module free of logging
//! concerns and makes the quantile math independently testable.

// Module-level cfg(debug_assertions) is attached at the mod declaration
// in lib.rs; do not duplicate it here.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Capacity of the rolling frame-time window. Sized so the 99th percentile
/// at 60fps has a full second plus headroom of samples to draw from.
const WINDOW_CAPACITY: usize = 240;

/// Quantile summary produced by [`FrameProbe::snapshot`]. Durations are
/// stored as microseconds so a formatted log line can emit them without
/// reallocating.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameQuantiles {
    pub count: u32,
    pub min_us: u64,
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub max_us: u64,
}

/// Ring buffer that accumulates frame durations and emits quantile
/// summaries at most once per reporting interval (default 1 second).
///
/// Deliberately simple: we do not maintain an ordered structure, we
/// copy-and-sort the window at emit time. With a 240-slot window that
/// cost is negligible compared to the frame it describes.
pub struct FrameProbe {
    /// Rolling window of frame durations in microseconds. A `VecDeque` lets
    /// eviction of the oldest sample run in O(1) instead of the O(n) shift a
    /// `Vec::remove(0)` would incur.
    samples: VecDeque<u64>,
    /// Time the last summary was emitted. `None` until the first emit.
    last_emit: Option<Instant>,
    /// Interval between emitted summaries.
    report_interval: Duration,
}

impl Default for FrameProbe {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameProbe {
    /// Build a probe with the default one-second reporting interval.
    pub fn new() -> Self {
        Self {
            samples: VecDeque::with_capacity(WINDOW_CAPACITY),
            last_emit: None,
            report_interval: Duration::from_secs(1),
        }
    }

    /// Build a probe with a custom reporting interval. Tests use this so
    /// they do not need to wait a full second.
    pub fn with_report_interval(report_interval: Duration) -> Self {
        Self { samples: VecDeque::with_capacity(WINDOW_CAPACITY), last_emit: None, report_interval }
    }

    /// Number of samples currently in the window.
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    /// Record a single frame duration. When the ring is full the oldest
    /// sample is evicted, so the window always reflects the most recent
    /// `WINDOW_CAPACITY` frames.
    pub fn record_frame(&mut self, frame_time: Duration) {
        if self.samples.len() == WINDOW_CAPACITY {
            self.samples.pop_front();
        }
        self.samples.push_back(frame_time.as_micros() as u64);
    }

    /// Returns `Some(summary)` if the reporting interval has elapsed and at
    /// least one frame has been recorded. Resets the interval counter as a
    /// side effect so callers can treat the return value as a trigger.
    ///
    /// Returns `None` when either there is nothing to report or the
    /// interval has not elapsed yet.
    pub fn maybe_emit(&mut self, now: Instant) -> Option<FrameQuantiles> {
        if self.samples.is_empty() {
            return None;
        }
        let due = match self.last_emit {
            None => true,
            Some(prev) => now.saturating_duration_since(prev) >= self.report_interval,
        };
        if !due {
            return None;
        }
        self.last_emit = Some(now);
        Some(self.snapshot())
    }

    /// Compute a quantile snapshot from the current window without
    /// mutating the window. Exposed primarily for tests.
    pub fn snapshot(&self) -> FrameQuantiles {
        if self.samples.is_empty() {
            return FrameQuantiles::default();
        }
        let mut sorted: Vec<u64> = self.samples.iter().copied().collect();
        sorted.sort_unstable();
        let n = sorted.len();
        FrameQuantiles {
            count: n as u32,
            min_us: sorted[0],
            p50_us: percentile(&sorted, 0.50),
            p95_us: percentile(&sorted, 0.95),
            p99_us: percentile(&sorted, 0.99),
            max_us: sorted[n - 1],
        }
    }
}

/// Nearest-rank percentile over a pre-sorted slice. Returns 0 for an
/// empty slice (callers guard against this case but defensive code is
/// cheap).
fn percentile(sorted: &[u64], q: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let n = sorted.len();
    // Nearest-rank: ceil(q * n), 1-indexed, clamp to [1, n].
    let rank = (q * n as f64).ceil() as usize;
    let idx = rank.saturating_sub(1).min(n - 1);
    sorted[idx]
}

impl std::fmt::Display for FrameQuantiles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "frames={} min={:.2}ms p50={:.2}ms p95={:.2}ms p99={:.2}ms max={:.2}ms",
            self.count,
            self.min_us as f64 / 1000.0,
            self.p50_us as f64 / 1000.0,
            self.p95_us as f64 / 1000.0,
            self.p99_us as f64 / 1000.0,
            self.max_us as f64 / 1000.0,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_probe_emits_nothing() {
        let mut probe = FrameProbe::with_report_interval(Duration::from_millis(10));
        assert!(probe.maybe_emit(Instant::now()).is_none());
    }

    #[test]
    fn snapshot_reports_quantiles_for_ten_frames() {
        let mut probe = FrameProbe::with_report_interval(Duration::from_secs(1));
        for ms in 1..=10 {
            probe.record_frame(Duration::from_millis(ms));
        }
        let snap = probe.snapshot();
        assert_eq!(snap.count, 10);
        assert_eq!(snap.min_us, 1_000);
        assert_eq!(snap.max_us, 10_000);
        // Nearest-rank: p50 of 10 samples is index ceil(5)-1 = 4 -> 5ms.
        assert_eq!(snap.p50_us, 5_000);
        // p95: ceil(9.5)-1 = 9 -> 10ms.
        assert_eq!(snap.p95_us, 10_000);
        // p99: ceil(9.9)-1 = 9 -> 10ms.
        assert_eq!(snap.p99_us, 10_000);
    }

    #[test]
    fn maybe_emit_waits_for_interval() {
        let mut probe = FrameProbe::with_report_interval(Duration::from_millis(100));
        probe.record_frame(Duration::from_millis(5));
        let t0 = Instant::now();
        // First emit always fires because last_emit is None.
        assert!(probe.maybe_emit(t0).is_some());
        // Second emit too soon -> None.
        assert!(probe.maybe_emit(t0 + Duration::from_millis(50)).is_none());
        // After interval -> Some.
        assert!(probe.maybe_emit(t0 + Duration::from_millis(100)).is_some());
    }

    #[test]
    fn window_caps_at_capacity() {
        let mut probe = FrameProbe::with_report_interval(Duration::from_secs(1));
        for i in 0..(WINDOW_CAPACITY + 50) {
            probe.record_frame(Duration::from_micros(i as u64));
        }
        assert_eq!(probe.sample_count(), WINDOW_CAPACITY);
    }

    #[test]
    fn percentile_handles_single_sample() {
        assert_eq!(percentile(&[42], 0.50), 42);
        assert_eq!(percentile(&[42], 0.99), 42);
    }

    #[test]
    fn percentile_empty_returns_zero() {
        assert_eq!(percentile(&[], 0.50), 0);
    }

    #[test]
    fn display_format_is_stable() {
        let q = FrameQuantiles {
            count: 60,
            min_us: 1_000,
            p50_us: 5_000,
            p95_us: 12_000,
            p99_us: 18_500,
            max_us: 25_000,
        };
        let s = q.to_string();
        assert!(s.contains("frames=60"));
        assert!(s.contains("min=1.00ms"));
        assert!(s.contains("p50=5.00ms"));
        assert!(s.contains("p95=12.00ms"));
        assert!(s.contains("p99=18.50ms"));
        assert!(s.contains("max=25.00ms"));
    }

    #[test]
    fn snapshot_of_empty_probe_is_all_zero() {
        let probe = FrameProbe::new();
        let snap = probe.snapshot();
        assert_eq!(snap, FrameQuantiles::default());
    }
}
