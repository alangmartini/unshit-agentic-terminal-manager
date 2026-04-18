//! Bench harness for perf regression #52.
//!
//! Invoked via `--bench <mode>` CLI flag. Injects a canned workload into
//! the first spawned PTY, feeds `on_frame_metrics` samples into a ring
//! buffer, writes a JSON report, and force-exits the process.
//!
//! Modes:
//! - `dir-loop`: writes `dir\r\n` every 80ms. Stresses scroll-heavy output.
//! - `type-burst`: writes one ASCII char every 50ms. Stresses the
//!   single-cell keystroke path.
//!
//! The probe is off by default; activation is gated on the main thread
//! setting up the bench. In non-bench runs, `record_frame` is a
//! no-op lock+return.
//!
//! This module owns no framework code. It hooks via the public
//! `AppConfig::on_frame_metrics` callback and the already-existing
//! `PtyManager::write` API.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use unshit::app::FrameMetrics;
#[cfg(feature = "input-latency-histogram")]
use unshit::app::InputLatencySnapshot;

use crate::state::SharedState;

#[derive(Clone, Copy, Debug)]
pub enum BenchMode {
    DirLoop,
    TypeBurst,
}

impl BenchMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "dir-loop" => Some(Self::DirLoop),
            "type-burst" => Some(Self::TypeBurst),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::DirLoop => "dir-loop",
            Self::TypeBurst => "type-burst",
        }
    }
}

pub struct BenchConfig {
    pub mode: BenchMode,
    pub duration: Duration,
    pub warmup: Duration,
    pub out_path: PathBuf,
}

struct BenchState {
    samples_us: VecDeque<u64>,
    tree_build_us_sum: u64,
    layout_us_sum: u64,
    batch_us_sum: u64,
    gpu_us_sum: u64,
    frames: u64,
    /// Most recent pacer coalescing interval, in nanoseconds. Captured
    /// from [`FrameMetrics::pacer_min_interval_ns`] each frame. Constant
    /// across the bench on a stationary window; changes if the user
    /// drags the window to a different monitor mid-run (uncommon but
    /// supported).
    last_pacer_min_interval_ns: u64,
    active: bool,
    #[cfg(feature = "input-latency-histogram")]
    latency_snapshot: Option<InputLatencySnapshot>,
}

impl BenchState {
    const fn new() -> Self {
        Self {
            samples_us: VecDeque::new(),
            tree_build_us_sum: 0,
            layout_us_sum: 0,
            batch_us_sum: 0,
            gpu_us_sum: 0,
            frames: 0,
            last_pacer_min_interval_ns: 0,
            active: false,
            #[cfg(feature = "input-latency-histogram")]
            latency_snapshot: None,
        }
    }
}

static STATE: OnceLock<Mutex<BenchState>> = OnceLock::new();

fn state() -> &'static Mutex<BenchState> {
    STATE.get_or_init(|| Mutex::new(BenchState::new()))
}

/// Called once per rendered frame by the framework. No-op unless the
/// bench thread has flipped `active` on.
pub fn record_frame(m: &FrameMetrics) {
    let mut s = state().lock().unwrap();
    if !s.active {
        return;
    }
    s.samples_us.push_back(m.total_us);
    s.tree_build_us_sum += m.tree_build_us;
    s.layout_us_sum += m.layout_us;
    s.batch_us_sum += m.batch_build_us;
    s.gpu_us_sum += m.gpu_render_us;
    s.last_pacer_min_interval_ns = m.pacer_min_interval_ns;
    s.frames += 1;
}

/// Called once per rendered frame by the framework when the
/// `input-latency-histogram` feature is enabled. Stores the latest
/// snapshot so `build_report` can read it without polling. No-op
/// unless the bench is active.
#[cfg(feature = "input-latency-histogram")]
pub fn record_input_latency(snap: &InputLatencySnapshot) {
    let mut s = state().lock().unwrap();
    if !s.active {
        return;
    }
    s.latency_snapshot = Some(snap.clone());
}

fn activate() {
    let mut s = state().lock().unwrap();
    s.active = true;
    s.samples_us.clear();
    s.tree_build_us_sum = 0;
    s.layout_us_sum = 0;
    s.batch_us_sum = 0;
    s.gpu_us_sum = 0;
    s.frames = 0;
    #[cfg(feature = "input-latency-histogram")]
    {
        s.latency_snapshot = None;
    }
    s.last_pacer_min_interval_ns = 0;
}

fn deactivate() {
    let mut s = state().lock().unwrap();
    s.active = false;
}

#[derive(Serialize)]
struct Report {
    mode: &'static str,
    duration_s: f64,
    frames: u64,
    fps_mean: f64,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    max_ms: f64,
    tree_build_ms_avg: f64,
    layout_ms_avg: f64,
    batch_ms_avg: f64,
    gpu_ms_avg: f64,
    #[cfg(feature = "input-latency-histogram")]
    input_latency_p50_us: f64,
    #[cfg(feature = "input-latency-histogram")]
    input_latency_p95_us: f64,
    #[cfg(feature = "input-latency-histogram")]
    input_latency_p99_us: f64,
    #[cfg(feature = "input-latency-histogram")]
    input_latency_max_us: f64,
    #[cfg(feature = "input-latency-histogram")]
    events_per_frame_p50: f64,
    #[cfg(feature = "input-latency-histogram")]
    events_per_frame_p95: f64,
    #[cfg(feature = "input-latency-histogram")]
    events_per_frame_max: u64,
    #[cfg(feature = "input-latency-histogram")]
    mid_draw_events_dropped: u64,
    /// Frame pacer coalescing interval in milliseconds, derived from the
    /// active monitor's refresh rate. 8.0 on the historic fallback path;
    /// ~6.944 on 144Hz, ~4.166 on 240Hz.
    pacer_min_interval_ms: f64,
}

fn build_report(mode: BenchMode, elapsed: Duration) -> Report {
    let s = state().lock().unwrap();
    let n = s.samples_us.len();
    let mut sorted: Vec<u64> = s.samples_us.iter().copied().collect();
    sorted.sort_unstable();
    let pct = |q: f64| -> u64 {
        if sorted.is_empty() {
            return 0;
        }
        let rank = (q * n as f64).ceil() as usize;
        sorted[rank.saturating_sub(1).min(n - 1)]
    };
    let avg_ms = |sum: u64| -> f64 {
        if s.frames == 0 {
            0.0
        } else {
            sum as f64 / s.frames as f64 / 1000.0
        }
    };
    let fps = if elapsed.as_secs_f64() > 0.0 {
        s.frames as f64 / elapsed.as_secs_f64()
    } else {
        0.0
    };
    #[cfg(feature = "input-latency-histogram")]
    let (lat_p50, lat_p95, lat_p99, lat_max, ev_p50, ev_p95, ev_max, mid_draw) = {
        if let Some(ref snap) = s.latency_snapshot {
            let lat = &snap.latency_ns;
            let ev = &snap.events_per_frame;
            let us = |ns: u64| (ns as f64) / 1000.0;
            (
                us(lat.value_at_quantile(0.50)),
                us(lat.value_at_quantile(0.95)),
                us(lat.value_at_quantile(0.99)),
                us(lat.max()),
                ev.value_at_quantile(0.50) as f64,
                ev.value_at_quantile(0.95) as f64,
                ev.max(),
                snap.mid_draw_events_dropped,
            )
        } else {
            (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0, 0)
        }
    };

    Report {
        mode: mode.as_str(),
        duration_s: elapsed.as_secs_f64(),
        frames: s.frames,
        fps_mean: fps,
        p50_ms: pct(0.50) as f64 / 1000.0,
        p95_ms: pct(0.95) as f64 / 1000.0,
        p99_ms: pct(0.99) as f64 / 1000.0,
        max_ms: sorted.last().copied().unwrap_or(0) as f64 / 1000.0,
        tree_build_ms_avg: avg_ms(s.tree_build_us_sum),
        layout_ms_avg: avg_ms(s.layout_us_sum),
        batch_ms_avg: avg_ms(s.batch_us_sum),
        gpu_ms_avg: avg_ms(s.gpu_us_sum),
        #[cfg(feature = "input-latency-histogram")]
        input_latency_p50_us: lat_p50,
        #[cfg(feature = "input-latency-histogram")]
        input_latency_p95_us: lat_p95,
        #[cfg(feature = "input-latency-histogram")]
        input_latency_p99_us: lat_p99,
        #[cfg(feature = "input-latency-histogram")]
        input_latency_max_us: lat_max,
        #[cfg(feature = "input-latency-histogram")]
        events_per_frame_p50: ev_p50,
        #[cfg(feature = "input-latency-histogram")]
        events_per_frame_p95: ev_p95,
        #[cfg(feature = "input-latency-histogram")]
        events_per_frame_max: ev_max,
        #[cfg(feature = "input-latency-histogram")]
        mid_draw_events_dropped: mid_draw,
        pacer_min_interval_ms: s.last_pacer_min_interval_ns as f64 / 1_000_000.0,
    }
}

fn first_pane_id(shared: &SharedState) -> Option<u32> {
    let guard = shared.lock().ok()?;
    guard.terminals.keys().copied().next()
}

fn write_pty(shared: &SharedState, pane_id: u32, data: &[u8]) {
    if let Ok(mut guard) = shared.lock() {
        let _ = guard.pty_manager.write(pane_id, data);
    }
}

/// Spawn the bench runner thread. Returns immediately; the thread
/// sleeps for `warmup`, runs the workload for `duration`, writes the
/// report, then force-exits the process.
pub fn start(config: BenchConfig, shared: SharedState) {
    std::thread::spawn(move || {
        log::info!(
            "[bench] warmup={:?} mode={} duration={:?} out={:?}",
            config.warmup,
            config.mode.as_str(),
            config.duration,
            config.out_path
        );
        std::thread::sleep(config.warmup);

        let pane_id = match first_pane_id(&shared) {
            Some(id) => id,
            None => {
                log::error!("[bench] no terminal available after warmup; exiting");
                std::process::exit(2);
            }
        };

        log::info!("[bench] activated; driving pane {}", pane_id);
        activate();
        let t_start = Instant::now();

        match config.mode {
            BenchMode::DirLoop => run_dir_loop(&shared, pane_id, config.duration),
            BenchMode::TypeBurst => run_type_burst(&shared, pane_id, config.duration),
        }

        let elapsed = t_start.elapsed();
        deactivate();

        let report = build_report(config.mode, elapsed);
        let json = serde_json::to_string_pretty(&report).unwrap();
        log::info!(
            "[bench] done frames={} fps={:.1} p50={:.2}ms p95={:.2}ms p99={:.2}ms max={:.2}ms pacer={:.3}ms",
            report.frames,
            report.fps_mean,
            report.p50_ms,
            report.p95_ms,
            report.p99_ms,
            report.max_ms,
            report.pacer_min_interval_ms,
        );
        #[cfg(feature = "input-latency-histogram")]
        log::info!(
            "[bench] input latency p50={:.2}us p95={:.2}us p99={:.2}us max={:.2}us ev/frame p50={:.0} p95={:.0} mid_draw_dropped={}",
            report.input_latency_p50_us,
            report.input_latency_p95_us,
            report.input_latency_p99_us,
            report.input_latency_max_us,
            report.events_per_frame_p50,
            report.events_per_frame_p95,
            report.mid_draw_events_dropped,
        );

        if let Err(e) = std::fs::write(&config.out_path, &json) {
            log::error!("[bench] failed to write {:?}: {}", config.out_path, e);
        } else {
            log::info!("[bench] wrote {:?}", config.out_path);
        }
        println!("{}", json);

        // Mirror the on_close handler: kill child PTYs before exit so
        // they do not linger on Windows (issue #32 style ghost handles).
        if let Ok(mut guard) = shared.lock() {
            guard.pty_manager.destroy_all();
            guard.terminals.clear();
        }

        std::process::exit(0);
    });
}

fn run_dir_loop(shared: &SharedState, pane_id: u32, duration: Duration) {
    let interval = Duration::from_millis(80);
    let end = Instant::now() + duration;
    while Instant::now() < end {
        write_pty(shared, pane_id, b"dir\r\n");
        std::thread::sleep(interval);
    }
}

fn run_type_burst(shared: &SharedState, pane_id: u32, duration: Duration) {
    let interval = Duration::from_millis(50);
    let words = b"the quick brown fox jumps over the lazy dog ";
    let mut i = 0;
    let end = Instant::now() + duration;
    while Instant::now() < end {
        let byte = words[i % words.len()];
        write_pty(shared, pane_id, &[byte]);
        i += 1;
        std::thread::sleep(interval);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Test-only mutex that serializes every test in this module. The
    /// bench harness exposes a process-wide `STATE` mutex; running two
    /// tests in parallel would let one test's `activate` or
    /// `deactivate` call interleave with another test's assertion.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn guard() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn bench_mode_parse_known() {
        let _g = guard();
        assert!(matches!(
            BenchMode::parse("dir-loop"),
            Some(BenchMode::DirLoop)
        ));
        assert!(matches!(
            BenchMode::parse("type-burst"),
            Some(BenchMode::TypeBurst)
        ));
    }

    #[test]
    fn bench_mode_parse_unknown() {
        let _g = guard();
        assert!(BenchMode::parse("").is_none());
        assert!(BenchMode::parse("dir_loop").is_none());
        assert!(BenchMode::parse("DIR-LOOP").is_none());
    }

    #[test]
    fn record_frame_ignored_when_inactive() {
        let _g = guard();
        // State may have been touched by other tests; just assert
        // frames do not increment while inactive.
        deactivate();
        let before = state().lock().unwrap().frames;
        record_frame(&FrameMetrics {
            total_us: 5_000,
            ..Default::default()
        });
        let after = state().lock().unwrap().frames;
        assert_eq!(before, after);
    }

    #[test]
    fn record_frame_accumulates_when_active() {
        let _g = guard();
        activate();
        let before = state().lock().unwrap().frames;
        record_frame(&FrameMetrics {
            total_us: 5_000,
            tree_build_us: 1_000,
            layout_us: 2_000,
            batch_build_us: 1_000,
            gpu_render_us: 1_000,
            ..Default::default()
        });
        record_frame(&FrameMetrics {
            total_us: 10_000,
            ..Default::default()
        });
        let s = state().lock().unwrap();
        assert_eq!(s.frames, before + 2);
        drop(s);
        deactivate();
    }

    #[test]
    fn build_report_computes_percentiles() {
        let _g = guard();
        activate();
        {
            let mut s = state().lock().unwrap();
            s.samples_us.clear();
            s.frames = 0;
            s.tree_build_us_sum = 0;
            s.layout_us_sum = 0;
            s.batch_us_sum = 0;
            s.gpu_us_sum = 0;
            s.last_pacer_min_interval_ns = 0;
        }
        for ms in 1..=10u64 {
            record_frame(&FrameMetrics {
                total_us: ms * 1000,
                ..Default::default()
            });
        }
        let r = build_report(BenchMode::DirLoop, Duration::from_secs(1));
        assert_eq!(r.frames, 10);
        // Nearest-rank: p50 of 10 samples picks index ceil(5)-1 = 4 -> 5ms.
        assert!((r.p50_ms - 5.0).abs() < 0.001);
        assert!((r.max_ms - 10.0).abs() < 0.001);
        deactivate();
    }

    /// Issue #85 feature-off test: the JSON report must NOT contain
    /// any input latency keys when the cargo feature is absent.
    #[cfg(not(feature = "input-latency-histogram"))]
    #[test]
    fn bench_report_omits_latency_stats_when_feature_off() {
        let _g = guard();
        activate();
        let report = build_report(BenchMode::DirLoop, Duration::from_secs(1));
        deactivate();
        let json = serde_json::to_string(&report).unwrap();
        for key in [
            "input_latency_p50_us",
            "input_latency_p95_us",
            "input_latency_p99_us",
            "input_latency_max_us",
            "events_per_frame_p50",
            "events_per_frame_p95",
            "events_per_frame_max",
            "mid_draw_events_dropped",
        ] {
            assert!(
                !json.contains(key),
                "feature off but JSON contains {}: {}",
                key,
                json
            );
        }
    }

    /// Issue #85 feature-on test: the JSON report must contain the
    /// eight latency keys whenever the feature is active, even when
    /// zero input events were observed.
    #[cfg(feature = "input-latency-histogram")]
    #[test]
    fn bench_report_emits_latency_stats_when_feature_on() {
        let _g = guard();
        activate();
        let report = build_report(BenchMode::DirLoop, Duration::from_secs(1));
        deactivate();
        let json = serde_json::to_string(&report).unwrap();
        for key in [
            "input_latency_p50_us",
            "input_latency_p95_us",
            "input_latency_p99_us",
            "input_latency_max_us",
            "events_per_frame_p50",
            "events_per_frame_p95",
            "events_per_frame_max",
            "mid_draw_events_dropped",
        ] {
            assert!(
                json.contains(key),
                "feature on but JSON missing {}: {}",
                key,
                json
            );
        }
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let p50 = parsed
            .get("input_latency_p50_us")
            .and_then(|v| v.as_f64())
            .expect("input_latency_p50_us must be a finite number");
        assert!(p50.is_finite());
    }

    /// Issue #85 feature-on test: feeding a synthetic snapshot into
    /// `record_input_latency` makes `build_report` surface the
    /// recorded quantiles. Exercises the full recorder to report
    /// pipeline without needing a live event loop.
    #[cfg(feature = "input-latency-histogram")]
    #[test]
    fn bench_report_quantiles_reflect_recorded_snapshot() {
        use crate::bench::record_input_latency;
        use unshit::app::InputLatencyTracker;

        let _g = guard();
        activate();
        // Clear any residue from other tests.
        {
            let mut s = state().lock().unwrap();
            s.latency_snapshot = None;
        }

        let mut tracker = InputLatencyTracker::new().unwrap();
        let base = Instant::now();
        // Three frames of known latencies: 1ms, 2ms, 5ms.
        for &lat_ms in &[1u64, 2, 5] {
            tracker.record_event(base);
            tracker.record_frame_presented(base + Duration::from_millis(lat_ms));
        }
        let snap = tracker.snapshot();
        record_input_latency(&snap);

        let report = build_report(BenchMode::DirLoop, Duration::from_secs(1));
        deactivate();

        // p50 of [1,2,5] in hdrhistogram is the middle value, 2ms = 2000us.
        assert!(
            (report.input_latency_p50_us - 2000.0).abs() < 200.0,
            "p50 {} us not near 2000 us",
            report.input_latency_p50_us
        );
        assert!(
            report.input_latency_max_us >= 4900.0,
            "max {} us should reflect the 5ms sample",
            report.input_latency_max_us
        );
    }

    #[test]
    fn report_records_pacer_min_interval_from_last_frame() {
        let _g = guard();
        activate();
        {
            let mut s = state().lock().unwrap();
            s.samples_us.clear();
            s.frames = 0;
            s.tree_build_us_sum = 0;
            s.layout_us_sum = 0;
            s.batch_us_sum = 0;
            s.gpu_us_sum = 0;
            s.last_pacer_min_interval_ns = 0;
        }
        record_frame(&FrameMetrics {
            total_us: 5_000,
            pacer_min_interval_ns: 6_944_444,
            ..Default::default()
        });
        let r = build_report(BenchMode::DirLoop, Duration::from_secs(1));
        assert!(
            (r.pacer_min_interval_ms - 6.944_444).abs() < 0.001,
            "expected ~6.944ms, got {}",
            r.pacer_min_interval_ms
        );
        deactivate();
    }

    #[test]
    fn report_pacer_min_interval_is_zero_without_frames() {
        let _g = guard();
        activate();
        {
            let mut s = state().lock().unwrap();
            s.samples_us.clear();
            s.frames = 0;
            s.tree_build_us_sum = 0;
            s.layout_us_sum = 0;
            s.batch_us_sum = 0;
            s.gpu_us_sum = 0;
            s.last_pacer_min_interval_ns = 0;
        }
        let r = build_report(BenchMode::DirLoop, Duration::from_secs(1));
        assert_eq!(r.pacer_min_interval_ms, 0.0);
        deactivate();
    }
}
