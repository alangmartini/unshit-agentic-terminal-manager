//! In-app FPS overlay for the 120fps perf work (Phase 0).
//!
//! A small floating widget pinned to the top right corner, toggled by
//! `Ctrl+Shift+F`. Shows current fps, p50/p95/p99 frame times across a
//! rolling one second window, the per-stage breakdown from
//! [`unshit::app::FrameMetrics`], and the last frame's quad and glyph
//! counts. The overlay reads its data from a process-wide
//! [`FpsOverlayState`] populated by the framework's
//! `on_frame_metrics` callback, so building the widget is a pure
//! function of `FpsOverlayState` and stays cheap on the render path.
//!
//! Toggling the overlay on also flips the framework's `FrameProbe`
//! global emit flag so release builds start writing the once-per-second
//! `[FRAME] frames=... p50=...` log lines while the overlay is up.
//! Toggling off restores the build-default quiet behavior in release.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use unshit::app::FrameMetrics;
use unshit::core::element::*;

/// Capacity of the rolling frame-time window used by the overlay.
/// Sized to cover roughly one second at 240fps, matching the
/// framework's [`unshit::app::FrameProbe`] window so quantiles agree.
const WINDOW_CAPACITY: usize = 240;

/// Latest frame metrics plus a rolling window of `total_us` samples for
/// quantile computation. Cloned by the UI builder once per frame.
#[derive(Clone, Debug, Default)]
pub struct FpsOverlayState {
    pub visible: bool,
    pub last: FrameMetrics,
    /// Rolling window of `total_us` samples in microseconds. The
    /// VecDeque lets eviction of the oldest sample be O(1).
    pub samples_us: VecDeque<u64>,
    /// Wall-clock instant of the last recorded frame. Used to compute
    /// the window-based current fps without a system clock call inside
    /// the renderer.
    pub last_frame_at: Option<Instant>,
}

impl FpsOverlayState {
    fn new() -> Self {
        Self {
            visible: false,
            last: FrameMetrics::default(),
            samples_us: VecDeque::with_capacity(WINDOW_CAPACITY),
            last_frame_at: None,
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
        self.last = metrics.clone();
        self.last_frame_at = Some(now);
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

    /// Instantaneous fps derived from the most recent sample. Returns
    /// 0.0 when no frame has been recorded.
    pub fn current_fps(&self) -> f32 {
        let last = match self.samples_us.back() {
            Some(&us) if us > 0 => us,
            _ => return 0.0,
        };
        1_000_000.0 / last as f32
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

/// Called from the framework's `on_frame_metrics` callback every
/// rendered frame. Cheap enough to run unconditionally: a single mutex
/// acquisition, a clone of `FrameMetrics`, and a ring buffer push. The
/// overlay reads from this same state when it builds.
pub fn record_frame(metrics: &FrameMetrics) {
    let mut s = state().lock().unwrap_or_else(|p| p.into_inner());
    s.record(metrics, Instant::now());
}

/// Flip the overlay's visibility. Returns the new value. Side effect:
/// keeps the framework's [`unshit::app::frame_probe::set_emit_enabled`]
/// global in sync so the periodic `[FRAME]` log lines match the
/// overlay's on/off state.
pub fn toggle_visible() -> bool {
    let mut s = state().lock().unwrap_or_else(|p| p.into_inner());
    s.visible = !s.visible;
    unshit::app::frame_probe::set_emit_enabled(s.visible);
    s.visible
}

/// Snapshot the current overlay state without holding the mutex past
/// the clone. Used by the UI builder so the render closure does not
/// keep the lock during element construction.
pub fn snapshot() -> FpsOverlayState {
    state().lock().unwrap_or_else(|p| p.into_inner()).clone()
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
    let snap = snapshot();
    if !snap.visible {
        return ElementDef::new(Tag::Div).with_class("fps-overlay-hidden");
    }

    let q = snap.quantiles();
    let fps = snap.current_fps();
    let m = &snap.last;

    let mut card = ElementDef::new(Tag::Div)
        .with_class("fps-overlay")
        .with_id("fps-overlay");

    card = card
        .with_child(metric_row("fps", &format!("{:.1}", fps)))
        .with_child(metric_row("p50", &format_duration_us(q.p50_us)))
        .with_child(metric_row("p95", &format_duration_us(q.p95_us)))
        .with_child(metric_row("p99", &format_duration_us(q.p99_us)))
        .with_child(metric_row("frame", &format_duration_us(m.total_us)))
        .with_child(separator())
        .with_child(metric_row("tree", &format_duration_us(m.tree_build_us)))
        .with_child(metric_row("style", &format_duration_us(m.style_resolve_us)))
        .with_child(metric_row("layout", &format_duration_us(m.layout_us)))
        .with_child(metric_row("batch", &format_duration_us(m.batch_build_us)))
        .with_child(metric_row("gpu", &format_duration_us(m.gpu_render_us)))
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
    fn current_fps_inverts_last_sample_in_us() {
        let mut s = FpsOverlayState::new();
        // 8333us per frame -> ~120 fps.
        s.record(&metrics_with(8_333), Instant::now());
        let fps = s.current_fps();
        assert!((fps - 120.0).abs() < 0.5, "expected ~120 got {fps}");
    }

    #[test]
    fn current_fps_zero_for_empty_window() {
        let s = FpsOverlayState::new();
        assert_eq!(s.current_fps(), 0.0);
    }

    #[test]
    fn build_returns_hidden_div_when_off() {
        reset_for_test();
        let el = build_fps_overlay();
        assert!(el.classes.iter().any(|c| c == "fps-overlay-hidden"));
        assert!(el.children.is_empty());
    }

    #[test]
    fn toggle_visible_round_trip_flips_and_returns_new_value() {
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
        assert!(el.classes.iter().any(|c| c == "fps-overlay"));
        let row_count = el
            .children
            .iter()
            .filter(|c| c.classes.iter().any(|cls| cls == "fps-overlay-row"))
            .count();
        // 5 quantile rows + 5 stage rows + 4 counter rows = 14.
        assert_eq!(row_count, 14, "unexpected row count, got {row_count}");
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
        reset_for_test();
        record_frame(&metrics_with(4_000));
        let snap = snapshot();
        assert_eq!(snap.last.total_us, 4_000);
        assert_eq!(snap.samples_us.len(), 1);
        reset_for_test();
    }
}
