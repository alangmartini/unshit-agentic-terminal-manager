//! In-app FPS overlay for the 120fps perf work (Phase 0).
//!
//! A small floating widget pinned to the top right corner, toggled by
//! `Ctrl+Shift+F`. Shows the honest fps (paints within the trailing
//! second), p50/p95/p99 frame work times across a rolling window,
//! present-interval quantiles with a dropped-interval count, the
//! per-stage breakdown from [`unshit::app::FrameMetrics`], and the
//! last frame's quad and glyph counts. The overlay reads its data from
//! a process-wide [`FpsOverlayState`] populated by the framework's
//! `on_frame_metrics` callback, so building the widget is a pure
//! function of `FpsOverlayState` and stays cheap on the render path.
//!
//! Toggling the overlay on also flips the framework's `FrameProbe`
//! global emit flag so release builds start writing the once-per-second
//! `[FRAME] frames=... p50=...` log lines while the overlay is up.
//! Toggling off restores the build-default quiet behavior in release.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::json;
use unshit::app::FrameMetrics;
use unshit::core::element::*;

/// Capacity of the rolling frame-time window used by the overlay.
/// Sized to cover roughly one second at 240fps, matching the
/// framework's [`unshit::app::FrameProbe`] window so quantiles agree.
const WINDOW_CAPACITY: usize = 240;

/// Hard cap on the rolling paint-timestamp window backing the honest
/// fps computation. Sized to cover one second at 600fps so time-based
/// pruning, not the cap, normally bounds the window.
const FRAME_TIMES_CAPACITY: usize = 600;

/// Minimum gap between overlay-driven rebuild requests. Caps a visible
/// overlay's rebuild traffic at ~4Hz instead of one full rebuild per
/// painted frame (the observer effect).
const REBUILD_THROTTLE: Duration = Duration::from_millis(250);

/// Latest frame metrics plus a rolling window of `total_us` samples for
/// quantile computation. Cloned by the UI builder once per frame.
#[derive(Clone, Debug, Default)]
pub struct FpsOverlayState {
    pub visible: bool,
    pub last: FrameMetrics,
    /// Rolling window of `total_us` samples in microseconds. The
    /// VecDeque lets eviction of the oldest sample be O(1).
    pub samples_us: VecDeque<u64>,
    /// Monotonic counter incremented every time frame metrics are recorded.
    pub recorded_generation: u64,
    /// Monotonic counter incremented every time the visible overlay is built.
    pub rendered_generation: u64,
    /// Sample count shown by the most recent visible overlay build.
    pub rendered_sample_count: u32,
    /// Rolling window of paint-completion timestamps for the honest
    /// fps computation. Pruned to the trailing second relative to the
    /// newest entry on every record, with a hard cap of
    /// [`FRAME_TIMES_CAPACITY`].
    pub frame_times: VecDeque<Instant>,
    /// Rolling window of present intervals in microseconds
    /// (`FrameMetrics::present_interval_us`). Zero intervals (the
    /// first painted frame and idle cadence breaks, which the framework
    /// reports as 0) are not recorded.
    pub intervals_us: VecDeque<u64>,
    /// When the overlay last asked the host to rebuild. Throttles
    /// overlay-driven rebuild requests to [`REBUILD_THROTTLE`].
    pub last_rebuild_request: Option<Instant>,
}

impl FpsOverlayState {
    fn new() -> Self {
        Self {
            visible: false,
            last: FrameMetrics::default(),
            samples_us: VecDeque::with_capacity(WINDOW_CAPACITY),
            recorded_generation: 0,
            rendered_generation: 0,
            rendered_sample_count: 0,
            frame_times: VecDeque::with_capacity(FRAME_TIMES_CAPACITY),
            intervals_us: VecDeque::with_capacity(WINDOW_CAPACITY),
            last_rebuild_request: None,
        }
    }

    /// Record one frame. Always cheap: pop_front when full, push_back.
    /// Stores the metrics regardless of visibility so toggling on
    /// shows accurate quantiles immediately.
    pub fn record(&mut self, metrics: &FrameMetrics, now: Instant) {
        if self.samples_us.len() == WINDOW_CAPACITY {
            self.samples_us.pop_front();
        }
        self.samples_us.push_back(metrics.total_us);
        if metrics.present_interval_us > 0 {
            if self.intervals_us.len() == WINDOW_CAPACITY {
                self.intervals_us.pop_front();
            }
            self.intervals_us.push_back(metrics.present_interval_us);
        }
        if self.frame_times.len() == FRAME_TIMES_CAPACITY {
            self.frame_times.pop_front();
        }
        self.frame_times.push_back(now);
        while let Some(&oldest) = self.frame_times.front() {
            if now.saturating_duration_since(oldest) > Duration::from_secs(1) {
                self.frame_times.pop_front();
            } else {
                break;
            }
        }
        self.last = metrics.clone();
        self.recorded_generation = self.recorded_generation.saturating_add(1);
    }

    /// Quantile snapshot from the current window. Returns zeros for an
    /// empty window so the overlay can render a stable layout while
    /// waiting for the first frame after toggle on.
    pub fn quantiles(&self) -> FrameQuantiles {
        if self.samples_us.is_empty() {
            return FrameQuantiles::default();
        }
        let mut sorted: Vec<u64> = self.samples_us.iter().copied().collect();
        sorted.sort_unstable();
        let n = sorted.len();
        FrameQuantiles {
            count: n as u32,
            p50_us: percentile(&sorted, 0.50),
            p95_us: percentile(&sorted, 0.95),
            p99_us: percentile(&sorted, 0.99),
        }
    }

    /// Honest fps: painted frames within the trailing one-second
    /// window of recorded paint timestamps, divided by the span those
    /// timestamps actually cover. A pure function of recorded state
    /// (no clock reads) so tests can inject timestamps. Returns 0.0
    /// until two frames have landed inside the window.
    pub fn current_fps(&self) -> f32 {
        let newest = match self.frame_times.back() {
            Some(&t) => t,
            None => return 0.0,
        };
        let window_start = newest.checked_sub(Duration::from_secs(1));
        let mut count = 0usize;
        let mut oldest = newest;
        for &t in &self.frame_times {
            if window_start.map_or(true, |start| t >= start) {
                count += 1;
                if t < oldest {
                    oldest = t;
                }
            }
        }
        if count < 2 {
            return 0.0;
        }
        let span_s = newest.saturating_duration_since(oldest).as_secs_f32();
        if span_s <= 0.0 {
            return 0.0;
        }
        (count - 1) as f32 / span_s
    }

    /// Quantile snapshot of the present-interval window. Returns zeros
    /// while the window is empty (before the second painted frame).
    pub fn interval_quantiles(&self) -> IntervalQuantiles {
        if self.intervals_us.is_empty() {
            return IntervalQuantiles::default();
        }
        let mut sorted: Vec<u64> = self.intervals_us.iter().copied().collect();
        sorted.sort_unstable();
        IntervalQuantiles {
            p50_us: percentile(&sorted, 0.50),
            p95_us: percentile(&sorted, 0.95),
            p99_us: percentile(&sorted, 0.99),
            max_us: *sorted.last().unwrap(),
        }
    }

    /// Number of recorded present intervals longer than 1.5x the
    /// display period: frames whose paint missed at least one refresh.
    /// Falls back to the pacer interval when the display period is
    /// unknown, and to 0 when neither is known.
    pub fn dropped_count(&self) -> u32 {
        let period_us = if self.last.display_period_ns > 0 {
            self.last.display_period_ns / 1_000
        } else if self.last.pacer_min_interval_ns > 0 {
            self.last.pacer_min_interval_ns / 1_000
        } else {
            return 0;
        };
        let threshold_us = period_us + period_us / 2;
        self.intervals_us
            .iter()
            .filter(|&&us| us > threshold_us)
            .count() as u32
    }

    /// Recorded paint timestamps as negative milliseconds relative to
    /// the newest timestamp, newest last (offset 0.0), at most
    /// [`WINDOW_CAPACITY`] entries. The Phase 5 regression suite reads
    /// this from the diagnostics snapshot to study paint cadence.
    pub fn frame_offsets_ms(&self) -> Vec<f64> {
        let newest = match self.frame_times.back() {
            Some(&t) => t,
            None => return Vec::new(),
        };
        let skip = self.frame_times.len().saturating_sub(WINDOW_CAPACITY);
        self.frame_times
            .iter()
            .skip(skip)
            .map(|&t| {
                let ms = newest.saturating_duration_since(t).as_secs_f64() * 1_000.0;
                if ms == 0.0 {
                    0.0
                } else {
                    -ms
                }
            })
            .collect()
    }
}

/// Quantile snapshot for the overlay. Microseconds so the formatter
/// can decide its own display unit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameQuantiles {
    pub count: u32,
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
}

/// Quantile snapshot of the present-interval window, microseconds.
/// `max_us` is included because a single missed refresh matters even
/// when it does not move p99.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IntervalQuantiles {
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub max_us: u64,
}

fn percentile(sorted: &[u64], q: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let n = sorted.len();
    let rank = (q * n as f64).ceil() as usize;
    let idx = rank.saturating_sub(1).min(n - 1);
    sorted[idx]
}

static STATE: OnceLock<Mutex<FpsOverlayState>> = OnceLock::new();

fn state() -> &'static Mutex<FpsOverlayState> {
    STATE.get_or_init(|| Mutex::new(FpsOverlayState::new()))
}

#[cfg(test)]
static GLOBAL_STATE_TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
pub(crate) fn global_state_test_lock() -> std::sync::MutexGuard<'static, ()> {
    GLOBAL_STATE_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// Called from the framework's `on_frame_metrics` callback every
/// rendered frame. Cheap enough to run unconditionally: a single mutex
/// acquisition, a clone of `FrameMetrics`, and ring buffer pushes.
/// Returns true when the caller should request an overlay rebuild now:
/// the overlay is visible and no rebuild was requested within the last
/// [`REBUILD_THROTTLE`]. This caps overlay-driven rebuilds at ~4Hz so
/// a visible overlay no longer forces a full rebuild every painted
/// frame.
pub fn record_frame(metrics: &FrameMetrics) -> bool {
    let now = Instant::now();
    let mut s = state().lock().unwrap_or_else(|p| p.into_inner());
    s.record(metrics, now);
    if !s.visible {
        return false;
    }
    let due = s.last_rebuild_request.map_or(true, |at| {
        now.saturating_duration_since(at) >= REBUILD_THROTTLE
    });
    if due {
        s.last_rebuild_request = Some(now);
    }
    due
}

/// Flip the overlay's visibility. Returns the new value. Side effect:
/// keeps the framework's [`unshit::app::frame_probe::set_emit_enabled`]
/// global in sync so the periodic `[FRAME]` log lines match the
/// overlay's on/off state.
pub fn toggle_visible() -> bool {
    let mut s = state().lock().unwrap_or_else(|p| p.into_inner());
    s.visible = !s.visible;
    if s.visible {
        // A freshly shown overlay should rebuild on the next frame
        // instead of waiting out a stale throttle window.
        s.last_rebuild_request = None;
    }
    unshit::app::frame_probe::set_emit_enabled(s.visible);
    s.visible
}

/// Snapshot the current overlay state without holding the mutex past
/// the clone. Used by the UI builder so the render closure does not
/// keep the lock during element construction.
pub fn snapshot() -> FpsOverlayState {
    state().lock().unwrap_or_else(|p| p.into_inner()).clone()
}

fn mark_rendered(sample_count: u32) {
    let mut s = state().lock().unwrap_or_else(|p| p.into_inner());
    s.rendered_generation = s.rendered_generation.saturating_add(1);
    s.rendered_sample_count = sample_count;
}

pub fn diagnostic_json() -> serde_json::Value {
    let snap = snapshot();
    let q = snap.quantiles();
    let iq = snap.interval_quantiles();
    json!({
        "visible": snap.visible,
        "recorded_generation": snap.recorded_generation,
        "rendered_generation": snap.rendered_generation,
        "recorded_sample_count": q.count,
        "rendered_sample_count": snap.rendered_sample_count,
        "current_fps": snap.current_fps(),
        "last_total_us": snap.last.total_us,
        "interval_p50_us": iq.p50_us,
        "interval_p95_us": iq.p95_us,
        "interval_p99_us": iq.p99_us,
        "interval_max_us": iq.max_us,
        "dropped_count": snap.dropped_count(),
        "frame_offsets_ms": snap.frame_offsets_ms(),
    })
}

/// Format a microsecond duration for the overlay. Sub millisecond
/// values render in microseconds, otherwise milliseconds with two
/// decimals.
pub fn format_duration_us(us: u64) -> String {
    if us < 1_000 {
        format!("{}us", us)
    } else {
        format!("{:.2}ms", us as f64 / 1_000.0)
    }
}

/// Build the overlay element. Returns an empty hidden div when the
/// overlay is off so callers can include this unconditionally in their
/// root tree without a branching cost.
pub fn build_fps_overlay() -> ElementDef {
    // Check visibility before cloning: the full-state clone copies
    // three ring buffers (~13KB) under the global mutex, a cost a
    // permanently hidden overlay must not pay on every rebuild.
    let snap = {
        let s = state().lock().unwrap_or_else(|p| p.into_inner());
        if !s.visible {
            return ElementDef::new(Tag::Div).with_class("fps-overlay-hidden");
        }
        s.clone()
    };

    let q = snap.quantiles();
    mark_rendered(q.count);
    let fps = snap.current_fps();
    let iq = snap.interval_quantiles();
    let m = &snap.last;

    let mut card = ElementDef::new(Tag::Div)
        .with_class("fps-overlay")
        .with_id("fps-overlay");

    card = card
        .with_child(metric_row("fps", &format!("{:.1}", fps)))
        .with_child(metric_row("p50", &format_duration_us(q.p50_us)))
        .with_child(metric_row("p95", &format_duration_us(q.p95_us)))
        .with_child(metric_row("p99", &format_duration_us(q.p99_us)))
        .with_child(metric_row("work", &format_duration_us(m.total_us)))
        .with_child(separator())
        .with_child(metric_row("int p50", &format_duration_us(iq.p50_us)))
        .with_child(metric_row("int p95", &format_duration_us(iq.p95_us)))
        .with_child(metric_row("int p99", &format_duration_us(iq.p99_us)))
        .with_child(metric_row("int max", &format_duration_us(iq.max_us)))
        .with_child(metric_row("dropped", &snap.dropped_count().to_string()))
        .with_child(separator())
        .with_child(metric_row("tree", &format_duration_us(m.tree_build_us)))
        .with_child(metric_row("style", &format_duration_us(m.style_resolve_us)))
        .with_child(metric_row("layout", &format_duration_us(m.layout_us)))
        .with_child(metric_row("batch", &format_duration_us(m.batch_build_us)))
        // gpu_render_us is CPU encode/submit time, not GPU execution
        // time (no timestamp queries yet), so it must not be labeled
        // "gpu".
        .with_child(metric_row("encode", &format_duration_us(m.gpu_render_us)))
        .with_child(separator())
        .with_child(metric_row("quads", &m.quad_count.to_string()))
        .with_child(metric_row("glyphs", &m.glyph_count.to_string()))
        .with_child(metric_row(
            "atlas",
            &format!("{:.0}%", m.atlas_fill_ratio * 100.0),
        ))
        .with_child(metric_row("samples", &q.count.to_string()))
        .with_child(separator())
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("fps-overlay-hint")
                .with_text("fps overlay (Ctrl+Shift+F)".to_string()),
        );

    card
}

fn metric_row(label: &str, value: &str) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("fps-overlay-row")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("fps-overlay-label")
                .with_text(label.to_string()),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("fps-overlay-value")
                .with_text(value.to_string()),
        )
}

fn separator() -> ElementDef {
    ElementDef::new(Tag::Div).with_class("fps-overlay-sep")
}

/// Reset the overlay state to its default. Test only: lets each test
/// in this module start from a known empty window without observing
/// state from previous tests.
#[cfg(test)]
pub(crate) fn reset_for_test() {
    let mut s = state().lock().unwrap_or_else(|p| p.into_inner());
    *s = FpsOverlayState::new();
    unshit::app::frame_probe::set_emit_enabled(cfg!(debug_assertions));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics_with(total_us: u64) -> FrameMetrics {
        FrameMetrics {
            total_us,
            ..FrameMetrics::default()
        }
    }

    fn metrics_with_interval(present_interval_us: u64, display_period_ns: u64) -> FrameMetrics {
        FrameMetrics {
            present_interval_us,
            display_period_ns,
            ..FrameMetrics::default()
        }
    }

    #[test]
    fn format_duration_us_sub_millisecond_uses_us() {
        assert_eq!(format_duration_us(0), "0us");
        assert_eq!(format_duration_us(1), "1us");
        assert_eq!(format_duration_us(999), "999us");
    }

    #[test]
    fn format_duration_us_above_one_ms_uses_two_decimals() {
        // 8333us rounds to 8.33ms, the canonical 120fps frame budget.
        assert_eq!(format_duration_us(8_333), "8.33ms");
        assert_eq!(format_duration_us(1_000), "1.00ms");
        assert_eq!(format_duration_us(16_666), "16.67ms");
    }

    #[test]
    fn record_pushes_into_window_and_updates_last() {
        let mut s = FpsOverlayState::new();
        s.record(&metrics_with(8_333), Instant::now());
        assert_eq!(s.samples_us.len(), 1);
        assert_eq!(s.last.total_us, 8_333);
        assert_eq!(s.recorded_generation, 1);
    }

    #[test]
    fn record_evicts_oldest_when_window_full() {
        let mut s = FpsOverlayState::new();
        for i in 0..(WINDOW_CAPACITY + 5) {
            s.record(&metrics_with(i as u64), Instant::now());
        }
        assert_eq!(s.samples_us.len(), WINDOW_CAPACITY);
        // Oldest sample should be 5 (we pushed 0..245 with capacity 240).
        assert_eq!(*s.samples_us.front().unwrap(), 5);
    }

    #[test]
    fn quantiles_on_empty_window_are_all_zero() {
        let s = FpsOverlayState::new();
        assert_eq!(s.quantiles(), FrameQuantiles::default());
    }

    #[test]
    fn quantiles_match_nearest_rank_definition() {
        let mut s = FpsOverlayState::new();
        for ms in 1..=10 {
            s.record(&metrics_with(ms * 1_000), Instant::now());
        }
        let q = s.quantiles();
        assert_eq!(q.count, 10);
        // Nearest rank: p50 of 10 samples is index ceil(5)-1 = 4 -> 5ms.
        assert_eq!(q.p50_us, 5_000);
        assert_eq!(q.p95_us, 10_000);
        assert_eq!(q.p99_us, 10_000);
    }

    #[test]
    fn current_fps_counts_frames_in_trailing_second() {
        let mut s = FpsOverlayState::new();
        let base = Instant::now();
        // Timestamps 8.33ms apart -> ~120 fps regardless of total_us.
        for i in 0..121u64 {
            s.record(
                &metrics_with(50_000),
                base + Duration::from_micros(i * 8_333),
            );
        }
        let fps = s.current_fps();
        assert!((fps - 120.0).abs() < 1.0, "expected ~120 got {fps}");
    }

    #[test]
    fn current_fps_ignores_timestamps_outside_trailing_second() {
        let mut s = FpsOverlayState::new();
        let base = Instant::now();
        // A stale timestamp 2s before a 3-frame burst spaced 10ms apart
        // must not stretch the measured span.
        s.frame_times.push_back(base);
        s.frame_times.push_back(base + Duration::from_millis(2_000));
        s.frame_times.push_back(base + Duration::from_millis(2_010));
        s.frame_times.push_back(base + Duration::from_millis(2_020));
        let fps = s.current_fps();
        assert!((fps - 100.0).abs() < 0.1, "expected ~100 got {fps}");
    }

    #[test]
    fn current_fps_zero_for_empty_window() {
        let s = FpsOverlayState::new();
        assert_eq!(s.current_fps(), 0.0);
    }

    #[test]
    fn current_fps_zero_for_a_single_timestamp() {
        let mut s = FpsOverlayState::new();
        s.record(&metrics_with(8_333), Instant::now());
        assert_eq!(s.current_fps(), 0.0);
    }

    #[test]
    fn record_prunes_frame_times_older_than_one_second() {
        let mut s = FpsOverlayState::new();
        let base = Instant::now();
        s.record(&metrics_with(1), base);
        s.record(&metrics_with(1), base + Duration::from_millis(500));
        assert_eq!(s.frame_times.len(), 2);
        // Both prior timestamps fall out of the newest frame's window.
        s.record(&metrics_with(1), base + Duration::from_millis(2_000));
        assert_eq!(s.frame_times.len(), 1);
    }

    #[test]
    fn record_caps_frame_times_at_hard_cap() {
        let mut s = FpsOverlayState::new();
        // Identical timestamps never age out, so only the cap bounds them.
        let now = Instant::now();
        for _ in 0..(FRAME_TIMES_CAPACITY + 10) {
            s.record(&metrics_with(1), now);
        }
        assert_eq!(s.frame_times.len(), FRAME_TIMES_CAPACITY);
    }

    #[test]
    fn record_skips_zero_present_interval() {
        let mut s = FpsOverlayState::new();
        s.record(&metrics_with_interval(0, 0), Instant::now());
        assert!(s.intervals_us.is_empty());
        s.record(&metrics_with_interval(8_000, 0), Instant::now());
        assert_eq!(s.intervals_us.len(), 1);
        assert_eq!(*s.intervals_us.back().unwrap(), 8_000);
    }

    #[test]
    fn interval_quantiles_match_nearest_rank_with_max() {
        let mut s = FpsOverlayState::new();
        assert_eq!(s.interval_quantiles(), IntervalQuantiles::default());
        for ms in 1..=10 {
            s.record(&metrics_with_interval(ms * 1_000, 0), Instant::now());
        }
        let iq = s.interval_quantiles();
        assert_eq!(iq.p50_us, 5_000);
        assert_eq!(iq.p95_us, 10_000);
        assert_eq!(iq.p99_us, 10_000);
        assert_eq!(iq.max_us, 10_000);
    }

    #[test]
    fn dropped_count_uses_display_period() {
        let mut s = FpsOverlayState::new();
        // 120Hz panel: period 8333us, dropped threshold 12499us.
        for &us in &[8_000, 9_000, 13_000, 25_000] {
            s.record(&metrics_with_interval(us, 8_333_333), Instant::now());
        }
        assert_eq!(s.dropped_count(), 2);
    }

    #[test]
    fn dropped_count_falls_back_to_pacer_interval() {
        let mut s = FpsOverlayState::new();
        // No display period; pacer at 8ms -> threshold 12000us.
        for &us in &[11_999, 12_001] {
            let mut m = metrics_with_interval(us, 0);
            m.pacer_min_interval_ns = 8_000_000;
            s.record(&m, Instant::now());
        }
        assert_eq!(s.dropped_count(), 1);
    }

    #[test]
    fn dropped_count_zero_when_no_period_is_known() {
        let mut s = FpsOverlayState::new();
        s.record(&metrics_with_interval(1_000_000, 0), Instant::now());
        assert_eq!(s.dropped_count(), 0);
    }

    #[test]
    fn frame_offsets_ms_newest_last_capped_at_window() {
        let mut s = FpsOverlayState::new();
        assert!(s.frame_offsets_ms().is_empty());
        let base = Instant::now();
        for i in 0..300u64 {
            s.record(&metrics_with(1), base + Duration::from_millis(i));
        }
        let offsets = s.frame_offsets_ms();
        assert_eq!(offsets.len(), WINDOW_CAPACITY);
        // Newest last at offset 0; the oldest surviving entry is the
        // 60th timestamp, 239ms before the newest.
        assert_eq!(*offsets.last().unwrap(), 0.0);
        assert!((offsets[0] + 239.0).abs() < 0.01, "got {}", offsets[0]);
        assert!(offsets.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn build_returns_hidden_div_when_off() {
        let _guard = global_state_test_lock();
        reset_for_test();
        let el = build_fps_overlay();
        assert!(el.classes.iter().any(|c| c == "fps-overlay-hidden"));
        assert!(el.children.is_empty());
    }

    #[test]
    fn toggle_visible_round_trip_flips_and_returns_new_value() {
        let _guard = global_state_test_lock();
        reset_for_test();
        assert!(toggle_visible());
        assert!(snapshot().visible);
        assert!(unshit::app::frame_probe::is_emit_enabled());
        assert!(!toggle_visible());
        assert!(!snapshot().visible);
        assert!(!unshit::app::frame_probe::is_emit_enabled());
    }

    #[test]
    fn build_visible_overlay_includes_metric_rows_and_hint() {
        let _guard = global_state_test_lock();
        reset_for_test();
        toggle_visible();
        record_frame(&FrameMetrics {
            total_us: 8_333,
            tree_build_us: 100,
            style_resolve_us: 200,
            scale_us: 10,
            layout_us: 300,
            batch_build_us: 50,
            gpu_render_us: 400,
            quad_count: 128,
            glyph_count: 512,
            atlas_fill_ratio: 0.5,
            ..FrameMetrics::default()
        });
        let el = build_fps_overlay();
        let diagnostic = diagnostic_json();
        // Unrelated harness tests build the full UI (including this
        // overlay) without the module test lock, so a concurrent build
        // may bump rendered_generation past the one build above.
        assert!(diagnostic["rendered_generation"].as_u64().unwrap() >= 1);
        assert_eq!(diagnostic["rendered_sample_count"], 1);
        assert!(el.classes.iter().any(|c| c == "fps-overlay"));
        let row_count = el
            .children
            .iter()
            .filter(|c| c.classes.iter().any(|cls| cls == "fps-overlay-row"))
            .count();
        // 5 quantile rows + 5 interval rows + 5 stage rows + 4 counter
        // rows = 19.
        assert_eq!(row_count, 19, "unexpected row count, got {row_count}");
        let has_hint = el
            .children
            .iter()
            .any(|c| c.classes.iter().any(|cls| cls == "fps-overlay-hint"));
        assert!(has_hint, "expected hint row");
        // Cleanup so other tests start from a known state.
        reset_for_test();
    }

    #[test]
    fn record_frame_module_function_updates_global_state() {
        let _guard = global_state_test_lock();
        reset_for_test();
        assert!(!record_frame(&metrics_with(4_000)));
        let snap = snapshot();
        assert_eq!(snap.last.total_us, 4_000);
        assert_eq!(snap.samples_us.len(), 1);
        reset_for_test();
    }

    #[test]
    fn record_frame_throttles_rebuild_requests_when_visible() {
        let _guard = global_state_test_lock();
        reset_for_test();
        toggle_visible();
        // First frame after toggle requests a rebuild; every further
        // frame inside the throttle window does not, so a sustained
        // paint burst (a smooth scroll) sees at most ~4 overlay-driven
        // rebuilds per second ejecting it from the fast-paint path.
        assert!(record_frame(&metrics_with(4_000)));
        for _ in 0..20 {
            assert!(!record_frame(&metrics_with(4_000)));
        }
        assert!(snapshot().last_rebuild_request.is_some());
        reset_for_test();
    }

    #[test]
    fn diagnostic_json_exposes_overlay_counters() {
        let _guard = global_state_test_lock();
        reset_for_test();
        toggle_visible();
        record_frame(&metrics_with(4_000));
        record_frame(&metrics_with_interval(9_000, 8_333_333));
        build_fps_overlay();
        let diagnostic = diagnostic_json();
        assert_eq!(diagnostic["visible"], true);
        assert_eq!(diagnostic["recorded_generation"], 2);
        // >= because unrelated harness tests may build the visible
        // overlay concurrently (they do not take the module test lock).
        assert!(diagnostic["rendered_generation"].as_u64().unwrap() >= 1);
        assert_eq!(diagnostic["recorded_sample_count"], 2);
        assert_eq!(diagnostic["rendered_sample_count"], 2);
        assert_eq!(diagnostic["interval_p50_us"], 9_000);
        assert_eq!(diagnostic["interval_p95_us"], 9_000);
        assert_eq!(diagnostic["interval_p99_us"], 9_000);
        assert_eq!(diagnostic["interval_max_us"], 9_000);
        assert_eq!(diagnostic["dropped_count"], 0);
        let offsets = diagnostic["frame_offsets_ms"]
            .as_array()
            .expect("frame_offsets_ms must be an array");
        assert_eq!(offsets.len(), 2);
        assert_eq!(offsets[1], 0.0);
        reset_for_test();
    }
}
