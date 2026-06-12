//! Bench harness for perf regression #52.
//!
//! Invoked via `--bench <mode>` CLI flag. Injects a canned workload into
//! the first spawned PTY, feeds `on_frame_metrics` samples into a ring
//! buffer, writes a JSON report, and force-exits the process.
//!
//! Modes:
//! - `dir-loop`: writes `dir\r\n` every 80ms. Stresses scroll-heavy output.
//! - `stress-cat`: starts the PowerShell `stress-cat` profile function when
//!   available, falling back to the equivalent recursive System32 walk.
//! - `stress-cat-2pane` / `stress-cat-4pane`: creates live split panes and
//!   runs the same stress stream in every pane.
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
    StressCat,
    StressCat2Pane,
    StressCat4Pane,
    TypeBurst,
    /// Stress mode for issue #86 (epic #81 item 2): run the dir-loop
    /// workload at the hardware's sustained rate while reporting per
    /// frame counters for the instance buffer pool. Used to verify the
    /// pool is not a new hotspot under 120 fps class loads.
    InstancePoolStress,
}

impl BenchMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "dir-loop" => Some(Self::DirLoop),
            "stress-cat" => Some(Self::StressCat),
            "stress-cat-2pane" => Some(Self::StressCat2Pane),
            "stress-cat-4pane" => Some(Self::StressCat4Pane),
            "type-burst" => Some(Self::TypeBurst),
            "instance-pool-stress" => Some(Self::InstancePoolStress),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::DirLoop => "dir-loop",
            Self::StressCat => "stress-cat",
            Self::StressCat2Pane => "stress-cat-2pane",
            Self::StressCat4Pane => "stress-cat-4pane",
            Self::TypeBurst => "type-burst",
            Self::InstancePoolStress => "instance-pool-stress",
        }
    }

    fn stress_cat_pane_count(self) -> Option<usize> {
        match self {
            Self::StressCat => Some(1),
            Self::StressCat2Pane => Some(2),
            Self::StressCat4Pane => Some(4),
            _ => None,
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
    /// Present-to-present intervals in microseconds, one per painted
    /// frame after the first. Fed from [`FrameMetrics::present_interval_us`];
    /// zero samples (first frame of a run) are skipped so quantiles
    /// reflect real cadence gaps only.
    intervals_us: VecDeque<u64>,
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
    /// Most recent nonzero active-monitor refresh period, in
    /// nanoseconds, from [`FrameMetrics::display_period_ns`]. Stays 0
    /// when the refresh rate is unknown for the whole run; the judder
    /// threshold then falls back to `last_pacer_min_interval_ns`.
    last_display_period_ns: u64,
    active: bool,
    #[cfg(feature = "input-latency-histogram")]
    latency_snapshot: Option<InputLatencySnapshot>,
}

impl BenchState {
    const fn new() -> Self {
        Self {
            samples_us: VecDeque::new(),
            intervals_us: VecDeque::new(),
            tree_build_us_sum: 0,
            layout_us_sum: 0,
            batch_us_sum: 0,
            gpu_us_sum: 0,
            frames: 0,
            last_pacer_min_interval_ns: 0,
            last_display_period_ns: 0,
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
    if m.present_interval_us > 0 {
        s.intervals_us.push_back(m.present_interval_us);
    }
    if m.display_period_ns > 0 {
        s.last_display_period_ns = m.display_period_ns;
    }
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
    s.intervals_us.clear();
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
    s.last_display_period_ns = 0;
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
    /// Mean painted frames per second (frames / elapsed). This counts
    /// submitted paints, not displayed frames; cadence health lives in
    /// the `interval_*` and `judder_ratio` stats below.
    paints_per_sec_mean: f64,
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
    /// Minimum frame time observed in microseconds. Lower bound on the
    /// per frame cost of the pool below which hardware limits dominate.
    min_us: u64,
    /// 1st percentile frame time in milliseconds. Characterises the
    /// fastest path through the renderer where pool overhead matters.
    p01_ms: f64,
    /// Median present-to-present interval in milliseconds. Unlike the
    /// work-time quantiles above, these measure wall-clock paint
    /// cadence; a frame that wakes late shows up here, not in `p50_ms`.
    interval_p50_ms: f64,
    /// 95th percentile present interval in milliseconds.
    interval_p95_ms: f64,
    /// 99th percentile present interval in milliseconds.
    interval_p99_ms: f64,
    /// Largest present interval observed, in milliseconds.
    interval_max_ms: f64,
    /// Population standard deviation of present intervals, in
    /// milliseconds. A flat cadence has stddev near 0; the 8ms-vs-8.33ms
    /// beat shows up as nonzero spread even when the mean looks fine.
    interval_stddev_ms: f64,
    /// Fraction of present intervals longer than 1.5x the display
    /// period (a "dropped" cadence slot). Period comes from the active
    /// monitor's refresh rate, falling back to the pacer coalescing
    /// interval when the rate is unknown; 0.0 when neither is known or
    /// no intervals were recorded.
    judder_ratio: f64,
    /// Width of each `interval_histogram` bucket in milliseconds.
    interval_histogram_bucket_ms: f64,
    /// Present-interval histogram: counts per 0.5ms bucket starting at
    /// 0ms, the final bucket accumulating everything at or above 4x the
    /// display period (4x the pacer interval, then 4x 8.333ms, as
    /// fallbacks). Quantiles can only hint at the 8ms-vs-8.33ms beat;
    /// its bimodal shape is directly visible here. Empty when no
    /// intervals were recorded.
    interval_histogram: Vec<u64>,
}

/// Width of each [`Report::interval_histogram`] bucket in microseconds.
const INTERVAL_HISTOGRAM_BUCKET_US: u64 = 500;

/// Fixed-bucket histogram of present intervals. `result[i]` counts
/// intervals in `[i*500us, (i+1)*500us)`; the last bucket is an
/// overflow bucket for everything at or above 4x the reference period
/// (`period_ns`, or 8.333ms when unknown). Returns an empty Vec for no
/// intervals so the JSON report stays compact on runs without cadence
/// data.
fn interval_histogram(intervals_us: &[u64], period_ns: u64) -> Vec<u64> {
    if intervals_us.is_empty() {
        return Vec::new();
    }
    let reference_period_us = if period_ns > 0 {
        period_ns / 1_000
    } else {
        8_333
    };
    let cap_us = reference_period_us * 4;
    let bucket_count = (cap_us / INTERVAL_HISTOGRAM_BUCKET_US) as usize + 1;
    let mut buckets = vec![0u64; bucket_count];
    for &us in intervals_us {
        let idx = ((us / INTERVAL_HISTOGRAM_BUCKET_US) as usize).min(bucket_count - 1);
        buckets[idx] += 1;
    }
    buckets
}

/// Nearest-rank percentile over an ascending-sorted slice. Returns 0
/// for empty input.
fn percentile_us(sorted: &[u64], q: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let n = sorted.len();
    let rank = (q * n as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(n - 1)]
}

/// Population standard deviation of the samples, in the samples' own
/// unit. Returns 0.0 for empty input.
fn population_stddev(samples: &[u64]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let n = samples.len() as f64;
    let mean = samples.iter().map(|&v| v as f64).sum::<f64>() / n;
    let variance = samples
        .iter()
        .map(|&v| {
            let d = v as f64 - mean;
            d * d
        })
        .sum::<f64>()
        / n;
    variance.sqrt()
}

/// Fraction of present intervals strictly longer than 1.5x the display
/// period, i.e. paints that missed their cadence slot. `period_ns` is
/// the active monitor's refresh period (or the pacer fallback); 0.0
/// when no intervals were recorded or no period basis is known.
fn judder_ratio(intervals_us: &[u64], period_ns: u64) -> f64 {
    if intervals_us.is_empty() || period_ns == 0 {
        return 0.0;
    }
    let threshold_us = period_ns as f64 * 1.5 / 1000.0;
    let dropped = intervals_us
        .iter()
        .filter(|&&us| us as f64 > threshold_us)
        .count();
    dropped as f64 / intervals_us.len() as f64
}

fn build_report(mode: BenchMode, elapsed: Duration) -> Report {
    let s = state().lock().unwrap();
    let mut sorted: Vec<u64> = s.samples_us.iter().copied().collect();
    sorted.sort_unstable();
    let pct = |q: f64| -> u64 { percentile_us(&sorted, q) };
    let mut intervals_sorted: Vec<u64> = s.intervals_us.iter().copied().collect();
    intervals_sorted.sort_unstable();
    let judder_period_ns = if s.last_display_period_ns > 0 {
        s.last_display_period_ns
    } else {
        s.last_pacer_min_interval_ns
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
    let min_us = sorted.first().copied().unwrap_or(0);

    Report {
        mode: mode.as_str(),
        duration_s: elapsed.as_secs_f64(),
        frames: s.frames,
        paints_per_sec_mean: fps,
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
        min_us,
        p01_ms: pct(0.01) as f64 / 1000.0,
        interval_p50_ms: percentile_us(&intervals_sorted, 0.50) as f64 / 1000.0,
        interval_p95_ms: percentile_us(&intervals_sorted, 0.95) as f64 / 1000.0,
        interval_p99_ms: percentile_us(&intervals_sorted, 0.99) as f64 / 1000.0,
        interval_max_ms: intervals_sorted.last().copied().unwrap_or(0) as f64 / 1000.0,
        interval_stddev_ms: population_stddev(&intervals_sorted) / 1000.0,
        judder_ratio: judder_ratio(&intervals_sorted, judder_period_ns),
        interval_histogram_bucket_ms: INTERVAL_HISTOGRAM_BUCKET_US as f64 / 1000.0,
        interval_histogram: interval_histogram(&intervals_sorted, judder_period_ns),
    }
}

fn first_pane_id(shared: &SharedState) -> Option<u32> {
    let guard = shared.lock().ok()?;
    guard.terminals.keys().copied().next()
}

fn live_pane_ids(shared: &SharedState) -> Vec<u32> {
    let Ok(guard) = shared.lock() else {
        return Vec::new();
    };
    guard.panes.iter().flatten().map(|pane| pane.id.0).collect()
}

fn write_pty(shared: &SharedState, pane_id: u32, data: &[u8]) {
    if let Ok(mut guard) = shared.lock() {
        let _ = guard.pty_manager.write_blocking(pane_id, data);
    }
}

fn start_stress_cat(shared: &SharedState, pane_id: u32) {
    write_pty(shared, pane_id, STRESS_CAT_COMMAND.as_bytes());
}

fn create_split_stress_panes(shared: &SharedState, target_count: usize) -> Vec<u32> {
    let first = {
        let Ok(mut guard) = shared.lock() else {
            return Vec::new();
        };
        let first = guard.active_pane;
        match target_count {
            1 => {}
            2 => {
                crate::state::mutate_split_right(&mut guard, first);
            }
            4 => {
                crate::state::mutate_split_right(&mut guard, first);
                crate::state::mutate_split_down(&mut guard, first);
                let bottom_left = guard.active_pane;
                crate::state::mutate_split_right(&mut guard, bottom_left);
            }
            _ => {}
        }
        first.0
    };

    if target_count > 1 {
        write_pty(shared, first, b"echo split bench ready\r\n");
        std::thread::sleep(Duration::from_secs(1));
    }

    live_pane_ids(shared)
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

        let stress_cat_panes = if let Some(target_count) = config.mode.stress_cat_pane_count() {
            let panes = create_split_stress_panes(&shared, target_count);
            log::info!("[bench] prewarming stress-cat on panes {:?}", panes);
            for pane_id in &panes {
                start_stress_cat(&shared, *pane_id);
            }
            std::thread::sleep(Duration::from_secs(2));
            panes
        } else {
            Vec::new()
        };
        log::info!("[bench] activated; driving pane {}", pane_id);
        activate();
        let t_start = Instant::now();

        match config.mode {
            BenchMode::DirLoop => run_dir_loop(&shared, pane_id, config.duration),
            BenchMode::StressCat | BenchMode::StressCat2Pane | BenchMode::StressCat4Pane => {
                run_stress_cat(config.duration)
            }
            BenchMode::TypeBurst => run_type_burst(&shared, pane_id, config.duration),
            BenchMode::InstancePoolStress => {
                run_instance_pool_stress(&shared, pane_id, config.duration)
            }
        }
        drop(stress_cat_panes);

        let elapsed = t_start.elapsed();
        deactivate();

        let report = build_report(config.mode, elapsed);
        let json = serde_json::to_string_pretty(&report).unwrap();
        log::info!(
            "[bench] done frames={} paints/s={:.1} p50={:.2}ms p95={:.2}ms p99={:.2}ms max={:.2}ms pacer={:.3}ms int p99={:.2}ms judder={:.1}%",
            report.frames,
            report.paints_per_sec_mean,
            report.p50_ms,
            report.p95_ms,
            report.p99_ms,
            report.max_ms,
            report.pacer_min_interval_ms,
            report.interval_p99_ms,
            report.judder_ratio * 100.0,
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

#[cfg(windows)]
const STRESS_CAT_COMMAND: &str = concat!(
    r#"powershell.exe -NoLogo -ExecutionPolicy Bypass -Command "if (Get-Command stress-cat -CommandType Function -ErrorAction SilentlyContinue) { stress-cat } else { while ($true) { Get-ChildItem -Recurse C:\Windows\System32 -ErrorAction SilentlyContinue } }""#,
    "\r\n"
);

#[cfg(not(windows))]
const STRESS_CAT_COMMAND: &str =
    "while true; do find /usr/bin /bin /usr/local/bin 2>/dev/null; done\n";

fn run_stress_cat(duration: Duration) {
    std::thread::sleep(duration);
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

/// Stress mode for the instance buffer pool. Drives the same scroll
/// heavy workload as `dir-loop` but at a faster write cadence (10ms vs
/// 80ms) to produce more frames per second. The renderer acquires and
/// releases one pooled buffer per submission per frame; this variant
/// simply exercises the pool more aggressively and exposes any memory
/// leak or bind group churn that would otherwise only show up at 120
/// plus fps.
fn run_instance_pool_stress(shared: &SharedState, pane_id: u32, duration: Duration) {
    let interval = Duration::from_millis(10);
    let end = Instant::now() + duration;
    while Instant::now() < end {
        write_pty(shared, pane_id, b"dir\r\n");
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
            BenchMode::parse("stress-cat"),
            Some(BenchMode::StressCat)
        ));
        assert!(matches!(
            BenchMode::parse("stress-cat-2pane"),
            Some(BenchMode::StressCat2Pane)
        ));
        assert!(matches!(
            BenchMode::parse("stress-cat-4pane"),
            Some(BenchMode::StressCat4Pane)
        ));
        assert!(matches!(
            BenchMode::parse("type-burst"),
            Some(BenchMode::TypeBurst)
        ));
        assert!(matches!(
            BenchMode::parse("instance-pool-stress"),
            Some(BenchMode::InstancePoolStress)
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
        let r = build_report(BenchMode::DirLoop, Duration::from_secs(1));
        assert_eq!(r.pacer_min_interval_ms, 0.0);
        deactivate();
    }

    #[test]
    fn percentile_us_nearest_rank_and_empty() {
        let _g = guard();
        let sorted: Vec<u64> = (1..=10).collect();
        // Nearest-rank: p50 of 10 samples picks index ceil(5)-1 = 4 -> 5.
        assert_eq!(percentile_us(&sorted, 0.50), 5);
        assert_eq!(percentile_us(&sorted, 0.99), 10);
        assert_eq!(percentile_us(&[], 0.50), 0);
    }

    #[test]
    fn population_stddev_exact_on_hand_computed_input() {
        let _g = guard();
        // Classic example: mean 5, variance (9+1+1+1+0+0+4+16)/8 = 4.
        let samples = [2u64, 4, 4, 4, 5, 5, 7, 9];
        assert!((population_stddev(&samples) - 2.0).abs() < 1e-12);
        assert_eq!(population_stddev(&[]), 0.0);
        assert_eq!(population_stddev(&[7]), 0.0);
    }

    #[test]
    fn judder_ratio_counts_strictly_above_threshold() {
        let _g = guard();
        // period 8ms -> threshold exactly 12_000us. 12_000 is not
        // dropped (strict >); 12_001 and 20_000 are: 2 of 4 = 0.5.
        let intervals = [8_000u64, 12_000, 12_001, 20_000];
        assert!((judder_ratio(&intervals, 8_000_000) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn judder_ratio_is_zero_without_intervals_or_period() {
        let _g = guard();
        assert_eq!(judder_ratio(&[], 8_000_000), 0.0);
        assert_eq!(judder_ratio(&[20_000], 0), 0.0);
    }

    #[test]
    fn interval_histogram_buckets_expose_a_bimodal_beat() {
        let _g = guard();
        // 120Hz period: cap 33_332us -> 66 in-range buckets + overflow.
        let intervals = [8_333u64, 8_400, 16_666, 16_700, 100_000];
        let hist = interval_histogram(&intervals, 8_333_333);
        assert_eq!(hist.len(), 67);
        // The 8.33ms mode lands in bucket 16, the 16.67ms mode in 33.
        assert_eq!(hist[16], 2);
        assert_eq!(hist[33], 2);
        // Out-of-range samples accumulate in the overflow bucket.
        assert_eq!(*hist.last().unwrap(), 1);
        assert_eq!(hist.iter().sum::<u64>(), intervals.len() as u64);
    }

    #[test]
    fn interval_histogram_empty_input_and_unknown_period() {
        let _g = guard();
        assert!(interval_histogram(&[], 8_333_333).is_empty());
        // Unknown period falls back to a 8.333ms reference: same cap.
        let hist = interval_histogram(&[1_000], 0);
        assert_eq!(hist.len(), 67);
        assert_eq!(hist[2], 1);
    }

    #[test]
    fn record_frame_skips_zero_intervals_and_keeps_last_nonzero_period() {
        let _g = guard();
        activate();
        // First frame of a run reports interval 0; must not pollute the ring.
        record_frame(&FrameMetrics {
            total_us: 1_000,
            present_interval_us: 0,
            display_period_ns: 8_333_333,
            ..Default::default()
        });
        record_frame(&FrameMetrics {
            total_us: 1_000,
            present_interval_us: 8_333,
            // Period unknown this frame; the captured value must survive.
            display_period_ns: 0,
            ..Default::default()
        });
        let s = state().lock().unwrap();
        assert_eq!(s.intervals_us.len(), 1);
        assert_eq!(s.intervals_us[0], 8_333);
        assert_eq!(s.last_display_period_ns, 8_333_333);
        drop(s);
        deactivate();
    }

    #[test]
    fn report_interval_quantiles_stddev_and_judder() {
        let _g = guard();
        activate();
        // Four intervals: [8333, 8333, 8333, 16666]us at a 120Hz period
        // (threshold 12_500us): one dropped slot -> judder 0.25.
        for &interval in &[0u64, 8_333, 8_333, 8_333, 16_666] {
            record_frame(&FrameMetrics {
                total_us: 1_000,
                present_interval_us: interval,
                display_period_ns: 8_333_333,
                ..Default::default()
            });
        }
        let r = build_report(BenchMode::DirLoop, Duration::from_secs(1));
        deactivate();
        // Nearest-rank over 4 samples: p50 -> index 1, p95/p99 -> index 3.
        assert!((r.interval_p50_ms - 8.333).abs() < 0.001);
        assert!((r.interval_p95_ms - 16.666).abs() < 0.001);
        assert!((r.interval_p99_ms - 16.666).abs() < 0.001);
        assert!((r.interval_max_ms - 16.666).abs() < 0.001);
        // mean 10416.25us, variance 13_019_791.6875us^2, stddev 3608.295us.
        assert!(
            (r.interval_stddev_ms - 3.608_295).abs() < 0.001,
            "stddev {} ms",
            r.interval_stddev_ms
        );
        assert!((r.judder_ratio - 0.25).abs() < 1e-12);
    }

    #[test]
    fn report_judder_falls_back_to_pacer_interval_when_period_unknown() {
        let _g = guard();
        activate();
        // No display period all run; pacer 8ms -> threshold 12_000us.
        for &interval in &[8_000u64, 13_000] {
            record_frame(&FrameMetrics {
                total_us: 1_000,
                present_interval_us: interval,
                display_period_ns: 0,
                pacer_min_interval_ns: 8_000_000,
                ..Default::default()
            });
        }
        let r = build_report(BenchMode::DirLoop, Duration::from_secs(1));
        deactivate();
        assert!((r.judder_ratio - 0.5).abs() < 1e-12);
    }

    #[test]
    fn report_json_renames_fps_mean_to_paints_per_sec_mean() {
        let _g = guard();
        activate();
        let r = build_report(BenchMode::DirLoop, Duration::from_secs(1));
        deactivate();
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("paints_per_sec_mean"), "json: {}", json);
        assert!(!json.contains("fps_mean"), "json: {}", json);
        for key in [
            "interval_p50_ms",
            "interval_p95_ms",
            "interval_p99_ms",
            "interval_max_ms",
            "interval_stddev_ms",
            "judder_ratio",
            "interval_histogram",
            "interval_histogram_bucket_ms",
        ] {
            assert!(json.contains(key), "json missing {}: {}", key, json);
        }
    }
}
