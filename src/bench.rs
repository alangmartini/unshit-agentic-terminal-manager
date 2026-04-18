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
    active: bool,
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
            active: false,
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
    s.frames += 1;
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
            "[bench] done frames={} fps={:.1} p50={:.2}ms p95={:.2}ms p99={:.2}ms max={:.2}ms",
            report.frames,
            report.fps_mean,
            report.p50_ms,
            report.p95_ms,
            report.p99_ms,
            report.max_ms
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

    #[test]
    fn bench_mode_parse_known() {
        assert!(matches!(BenchMode::parse("dir-loop"), Some(BenchMode::DirLoop)));
        assert!(matches!(BenchMode::parse("type-burst"), Some(BenchMode::TypeBurst)));
    }

    #[test]
    fn bench_mode_parse_unknown() {
        assert!(BenchMode::parse("").is_none());
        assert!(BenchMode::parse("dir_loop").is_none());
        assert!(BenchMode::parse("DIR-LOOP").is_none());
    }

    #[test]
    fn record_frame_ignored_when_inactive() {
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
        activate();
        {
            let mut s = state().lock().unwrap();
            s.samples_us.clear();
            s.frames = 0;
            s.tree_build_us_sum = 0;
            s.layout_us_sum = 0;
            s.batch_us_sum = 0;
            s.gpu_us_sum = 0;
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
}
