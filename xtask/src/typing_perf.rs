use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::args::{TypingPerfMode, TypingPerfOpts};
use crate::desktop_regression::launcher::{AppLogFiles, AppSession};
use crate::desktop_regression::win32;
use crate::{binary_path, ensure_dir};

const BENCH_WARMUP_SECS: f64 = 3.0;
const BENCH_EXIT_GRACE: Duration = Duration::from_secs(6);
// Leave just enough time for the final key-up and terminal echo to paint before
// the benchmark thread takes its snapshot. A longer tail would measure idle
// time instead of the user-visible typing path.
const INPUT_SETTLE: Duration = Duration::from_millis(100);
const TARGET_WINDOW_WIDTH: i32 = 1280;
const TARGET_WINDOW_HEIGHT: i32 = 800;
const HUMAN_DELAYS_MS: &[u64] = &[62, 78, 55, 91, 69, 83, 48, 74, 101, 58, 87, 66];
const STRESS_DELAY_MS: u64 = 4;
const PROBE_COMMAND: &str = "echo terminal-manager typing cadence probe 0123456789";

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
enum InputProfile {
    Human,
    Stress,
}

impl InputProfile {
    fn label(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Stress => "stress",
        }
    }

    fn inter_key_delay(self, index: usize) -> Duration {
        match self {
            Self::Human => Duration::from_millis(HUMAN_DELAYS_MS[index % HUMAN_DELAYS_MS.len()]),
            Self::Stress => Duration::from_millis(STRESS_DELAY_MS),
        }
    }

    fn line_pause(self) -> Duration {
        match self {
            Self::Human => Duration::from_millis(140),
            Self::Stress => Duration::from_millis(8),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct BenchReport {
    mode: String,
    duration_s: f64,
    frames: u64,
    paints_per_sec_mean: f64,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    max_ms: f64,
    input_latency_p50_us: f64,
    input_latency_p95_us: f64,
    input_latency_p99_us: f64,
    input_latency_max_us: f64,
    input_latency_samples: u64,
    input_events_observed: u64,
    events_per_frame_p50: f64,
    events_per_frame_p95: f64,
    events_per_frame_max: u64,
    mid_draw_events_dropped: u64,
    pacer_min_interval_ms: f64,
    display_period_ms: f64,
    interval_p50_ms: f64,
    interval_p95_ms: f64,
    interval_p99_ms: f64,
    interval_max_ms: f64,
    interval_stddev_ms: f64,
    judder_ratio: f64,
}

#[derive(Debug, Serialize)]
struct PerfRun {
    correlation_id: String,
    iteration: u32,
    profile: InputProfile,
    characters_sent: u64,
    input_elapsed_ms: f64,
    report: BenchReport,
    verdict: TypingVerdict,
}

#[derive(Debug, Serialize)]
struct PerfSummary<'a> {
    schema_version: &'static str,
    correlation_id: &'a str,
    required_display_hz: f64,
    maximum_work_p99_us: u64,
    maximum_input_latency_p99_us: u64,
    minimum_input_latency_samples: u64,
    all_passed: bool,
    runs: &'a [PerfRun],
}

pub fn run(opts: &TypingPerfOpts) -> Result<(), String> {
    if !cfg!(target_os = "windows") {
        return Err("typing-perf requires a real Windows desktop".to_owned());
    }

    let workspace_root = workspace_root()?;
    let out_dir = absolute_from(&workspace_root, &opts.out_dir);
    ensure_dir(&out_dir).map_err(|e| format!("failed to create {}: {e}", out_dir.display()))?;
    if !opts.skip_build {
        prepare_release_binaries(&workspace_root)?;
    }
    let app_exe = binary_path(&workspace_root);
    let daemon_exe = app_exe.with_file_name(platform_daemon_name());
    if !app_exe.is_file() || !daemon_exe.is_file() {
        return Err(format!(
            "release binaries missing (app={}, daemon={})",
            app_exe.display(),
            daemon_exe.display()
        ));
    }

    let correlation_id = correlation_id();
    let profiles = selected_profiles(opts.mode);
    let mut runs = Vec::new();
    for iteration in 1..=opts.runs {
        for &profile in &profiles {
            runs.push(run_once(
                &workspace_root,
                &out_dir,
                &app_exe,
                &correlation_id,
                iteration,
                profile,
                opts.duration_secs,
            )?);
        }
    }

    let all_passed = runs.iter().all(|run| run.verdict.passed());
    let summary = PerfSummary {
        schema_version: "terminal-manager.typing-perf/v1",
        correlation_id: &correlation_id,
        required_display_hz: REQUIRED_DISPLAY_HZ,
        maximum_work_p99_us: MAXIMUM_WORK_P99_US,
        maximum_input_latency_p99_us: MAXIMUM_INPUT_LATENCY_P99_US,
        minimum_input_latency_samples: MINIMUM_INPUT_LATENCY_SAMPLES,
        all_passed,
        runs: &runs,
    };
    let summary_path = out_dir.join("summary.json");
    write_json(&summary_path, &summary)?;
    println!(
        "{}",
        serde_json::json!({
            "event": "typing_perf.complete",
            "correlation_id": correlation_id,
            "all_passed": all_passed,
            "runs": runs.len(),
            "summary_path": summary_path,
        })
    );

    if all_passed {
        Ok(())
    } else {
        let failures = runs.iter().filter(|run| !run.verdict.passed()).count();
        Err(format!(
            "typing performance failed {failures}/{} runs; see {}",
            runs.len(),
            summary_path.display()
        ))
    }
}

const REQUIRED_DISPLAY_HZ: f64 = 119.0;
const MINIMUM_DISPLAYED_FPS_RATIO: f64 = 0.97;
const MAXIMUM_WORK_P99_US: u64 = 8_000;
const MAXIMUM_INTERVAL_P99_PERIODS: f64 = 1.25;
const MAXIMUM_INTERVAL_PERIODS: f64 = 1.5;
const MAXIMUM_INPUT_LATENCY_P99_US: u64 = 16_667;
const MINIMUM_INPUT_LATENCY_SAMPLES: u64 = 50;

#[derive(Clone, Copy, Debug)]
struct TypingMetrics {
    display_period_ns: u64,
    current_fps: f64,
    work_p99_us: u64,
    interval_p99_us: u64,
    interval_max_us: u64,
    judder_ratio: f64,
    input_latency_p99_us: u64,
    input_latency_samples: u64,
}

#[derive(Clone, Copy, Debug, Serialize)]
struct TypingVerdict {
    display_hz: f64,
    renderer_capacity_fps: f64,
    minimum_displayed_fps: f64,
    display_supports_120: bool,
    renderer_meets_budget: bool,
    cadence_meets_budget: bool,
    input_meets_budget: bool,
}

impl TypingVerdict {
    fn passed(self) -> bool {
        self.display_supports_120
            && self.renderer_meets_budget
            && self.cadence_meets_budget
            && self.input_meets_budget
    }
}

fn analyze_metrics(metrics: TypingMetrics) -> TypingVerdict {
    let period_us = metrics.display_period_ns as f64 / 1_000.0;
    let display_hz = if metrics.display_period_ns == 0 {
        0.0
    } else {
        1_000_000_000.0 / metrics.display_period_ns as f64
    };
    let renderer_capacity_fps = if metrics.work_p99_us == 0 {
        0.0
    } else {
        1_000_000.0 / metrics.work_p99_us as f64
    };
    let minimum_displayed_fps = display_hz * MINIMUM_DISPLAYED_FPS_RATIO;
    let display_supports_120 = display_hz >= REQUIRED_DISPLAY_HZ;
    let renderer_meets_budget = metrics.work_p99_us < MAXIMUM_WORK_P99_US;
    let cadence_meets_budget = period_us > 0.0
        && metrics.current_fps >= minimum_displayed_fps
        && metrics.interval_p99_us as f64 <= period_us * MAXIMUM_INTERVAL_P99_PERIODS
        && metrics.interval_max_us as f64 <= period_us * MAXIMUM_INTERVAL_PERIODS
        && metrics.judder_ratio == 0.0;
    let input_meets_budget = metrics.input_latency_samples >= MINIMUM_INPUT_LATENCY_SAMPLES
        && metrics.input_latency_p99_us > 0
        && metrics.input_latency_p99_us <= MAXIMUM_INPUT_LATENCY_P99_US;

    TypingVerdict {
        display_hz,
        renderer_capacity_fps,
        minimum_displayed_fps,
        display_supports_120,
        renderer_meets_budget,
        cadence_meets_budget,
        input_meets_budget,
    }
}

fn selected_profiles(mode: TypingPerfMode) -> Vec<InputProfile> {
    match mode {
        TypingPerfMode::Human => vec![InputProfile::Human],
        TypingPerfMode::Stress => vec![InputProfile::Stress],
        TypingPerfMode::All => vec![InputProfile::Human, InputProfile::Stress],
    }
}

fn run_once(
    workspace_root: &Path,
    out_dir: &Path,
    app_exe: &Path,
    correlation_id: &str,
    iteration: u32,
    profile: InputProfile,
    duration_secs: u64,
) -> Result<PerfRun, String> {
    let run_name = format!("{}-{iteration}", profile.label());
    let run_dir = out_dir.join(&run_name);
    ensure_dir(&run_dir).map_err(|e| format!("failed to create {}: {e}", run_dir.display()))?;
    let logs = AppLogFiles::create(&run_dir, "typing-perf")?;
    let report_path = run_dir.join("bench.json");
    let args = vec![
        "--bench".to_owned(),
        "human-typing".to_owned(),
        "--duration".to_owned(),
        duration_secs.to_string(),
        "--warmup".to_owned(),
        BENCH_WARMUP_SECS.to_string(),
        "--out".to_owned(),
        report_path.display().to_string(),
    ];
    let mut env = BTreeMap::new();
    env.insert("RUST_LOG", "info".to_owned());

    println!(
        "{}",
        serde_json::json!({
            "event": "typing_perf.run_started",
            "correlation_id": correlation_id,
            "iteration": iteration,
            "profile": profile,
            "duration_secs": duration_secs,
        })
    );

    let mut session = AppSession::launch_with_logs_env_and_args(
        app_exe,
        workspace_root,
        Some(&logs),
        None,
        &env,
        &args,
    )?;
    let screen = win32::screen_size()?;
    let width = TARGET_WINDOW_WIDTH.min(screen.width.saturating_sub(80));
    let height = TARGET_WINDOW_HEIGHT.min(screen.height.saturating_sub(80));
    win32::set_window_rect(session.window(), 40, 40, width, height)?;
    win32::focus_window(session.window())?;
    // Exercise the exact terminal keyboard path during warmup. Besides priming
    // focus, this puts the probe glyphs in the atlas before measurement.
    win32::send_text_to_window(session.window(), PROBE_COMMAND)?;
    win32::send_enter_to_window(session.window())?;
    println!(
        "{}",
        serde_json::json!({
            "event": "typing_perf.input_path_primed",
            "correlation_id": correlation_id,
            "iteration": iteration,
            "profile": profile,
        })
    );
    wait_for_bench_activation(&logs, Duration::from_secs(10))?;

    let input_started = Instant::now();
    let input_duration = Duration::from_secs(duration_secs).saturating_sub(INPUT_SETTLE);
    let characters_sent = drive_typing(session.window(), profile, input_duration)?;
    let input_elapsed = input_started.elapsed();

    let status = session.wait_for_exit(BENCH_EXIT_GRACE)?;
    if !status.success() {
        return Err(format!(
            "typing benchmark process exited with {status}; logs are in {}",
            run_dir.display()
        ));
    }
    drop(session);

    let report_raw = std::fs::read_to_string(&report_path)
        .map_err(|e| format!("failed to read {}: {e}", report_path.display()))?;
    let report: BenchReport = serde_json::from_str(&report_raw)
        .map_err(|e| format!("invalid benchmark report {}: {e}", report_path.display()))?;
    if report.mode != "human-typing" {
        return Err(format!(
            "unexpected benchmark mode {:?} in {}",
            report.mode,
            report_path.display()
        ));
    }
    let metrics = metrics_from_report(&report);
    let verdict = analyze_metrics(metrics);
    println!(
        "{}",
        serde_json::json!({
            "event": "typing_perf.run_completed",
            "correlation_id": correlation_id,
            "iteration": iteration,
            "profile": profile,
            "characters_sent": characters_sent,
            "display_hz": verdict.display_hz,
            "paints_per_sec": report.paints_per_sec_mean,
            "work_p99_ms": report.p99_ms,
            "input_latency_p99_us": report.input_latency_p99_us,
            "input_latency_samples": report.input_latency_samples,
            "interval_p99_ms": report.interval_p99_ms,
            "judder_ratio": report.judder_ratio,
            "passed": verdict.passed(),
        })
    );

    Ok(PerfRun {
        correlation_id: correlation_id.to_owned(),
        iteration,
        profile,
        characters_sent,
        input_elapsed_ms: input_elapsed.as_secs_f64() * 1_000.0,
        report,
        verdict,
    })
}

fn drive_typing(
    window: win32::WindowHandle,
    profile: InputProfile,
    duration: Duration,
) -> Result<u64, String> {
    let deadline = Instant::now() + duration;
    let mut key_index = 0usize;
    let mut characters_sent = 0u64;
    while Instant::now() < deadline {
        for ch in PROBE_COMMAND.chars() {
            if Instant::now() >= deadline {
                return Ok(characters_sent);
            }
            let mut encoded = [0u8; 4];
            win32::send_text_to_window(window, ch.encode_utf8(&mut encoded))?;
            characters_sent = characters_sent.saturating_add(1);
            thread::sleep(profile.inter_key_delay(key_index));
            key_index = key_index.saturating_add(1);
        }
        if Instant::now() < deadline {
            win32::send_enter_to_window(window)?;
            characters_sent = characters_sent.saturating_add(1);
            thread::sleep(profile.line_pause());
        }
    }
    Ok(characters_sent)
}

fn wait_for_bench_activation(logs: &AppLogFiles, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        for path in [logs.stdout_path(), logs.stderr_path()] {
            if std::fs::read_to_string(path)
                .map(|contents| contents.contains("[bench] activated"))
                .unwrap_or(false)
            {
                return Ok(());
            }
        }
        thread::sleep(Duration::from_millis(25));
    }
    Err(format!(
        "benchmark did not activate within {:.1}s; inspect {} and {}",
        timeout.as_secs_f64(),
        logs.stdout_path().display(),
        logs.stderr_path().display()
    ))
}

fn metrics_from_report(report: &BenchReport) -> TypingMetrics {
    TypingMetrics {
        display_period_ns: (report.display_period_ms * 1_000_000.0).round() as u64,
        current_fps: report.paints_per_sec_mean,
        work_p99_us: (report.p99_ms * 1_000.0).round() as u64,
        interval_p99_us: (report.interval_p99_ms * 1_000.0).round() as u64,
        interval_max_us: (report.interval_max_ms * 1_000.0).round() as u64,
        judder_ratio: report.judder_ratio,
        input_latency_p99_us: report.input_latency_p99_us.round() as u64,
        input_latency_samples: report.input_latency_samples,
    }
}

fn prepare_release_binaries(workspace_root: &Path) -> Result<(), String> {
    run_status(
        Command::new("cargo")
            .args([
                "build",
                "--release",
                "--features",
                "input-latency-histogram",
                "--bin",
                "terminal-manager",
            ])
            .current_dir(workspace_root),
        "release app build",
    )?;
    run_status(
        Command::new("cargo")
            .args(["build", "--release", "-p", "unshit-ptyd"])
            .current_dir(workspace_root),
        "release daemon build",
    )
}

fn run_status(command: &mut Command, label: &str) -> Result<(), String> {
    println!(
        "{}",
        serde_json::json!({ "event": "typing_perf.command_started", "label": label })
    );
    let status = command
        .status()
        .map_err(|e| format!("failed to start {label}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{label} exited with {status}"))
    }
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let body = serde_json::to_string_pretty(value)
        .map_err(|e| format!("failed to serialize {}: {e}", path.display()))?;
    std::fs::write(path, body).map_err(|e| format!("failed to write {}: {e}", path.display()))
}

fn absolute_from(workspace_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        workspace_root.join(path)
    }
}

fn workspace_root() -> Result<PathBuf, String> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "xtask manifest directory has no workspace parent".to_owned())
}

fn platform_daemon_name() -> &'static str {
    if cfg!(windows) {
        "unshit-ptyd.exe"
    } else {
        "unshit-ptyd"
    }
}

fn correlation_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("typing-{millis}-{}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_budget_accepts_stable_120hz_cadence() {
        let verdict = analyze_metrics(TypingMetrics {
            display_period_ns: 8_333_333,
            current_fps: 120.0,
            work_p99_us: 2_100,
            interval_p99_us: 8_900,
            interval_max_us: 10_200,
            judder_ratio: 0.0,
            input_latency_p99_us: 5_200,
            input_latency_samples: 80,
        });

        assert!(verdict.passed());
        assert!(verdict.display_supports_120);
        assert!(verdict.renderer_meets_budget);
        assert!(verdict.cadence_meets_budget);
        assert!(verdict.input_meets_budget);
    }

    #[test]
    fn typing_budget_rejects_60hz_even_when_renderer_is_fast() {
        let verdict = analyze_metrics(TypingMetrics {
            display_period_ns: 16_666_666,
            current_fps: 60.0,
            work_p99_us: 1_900,
            interval_p99_us: 17_400,
            interval_max_us: 18_100,
            judder_ratio: 0.0,
            input_latency_p99_us: 12_000,
            input_latency_samples: 80,
        });

        assert!(!verdict.passed());
        assert!(!verdict.display_supports_120);
        assert!(verdict.renderer_meets_budget);
        assert!(verdict.cadence_meets_budget);
        assert!(verdict.input_meets_budget);
    }

    #[test]
    fn typing_budget_rejects_hitches_hidden_by_mean_fps() {
        let verdict = analyze_metrics(TypingMetrics {
            display_period_ns: 8_333_333,
            current_fps: 120.0,
            work_p99_us: 8_400,
            interval_p99_us: 16_700,
            interval_max_us: 25_100,
            judder_ratio: 0.02,
            input_latency_p99_us: 22_000,
            input_latency_samples: 80,
        });

        assert!(!verdict.passed());
        assert!(!verdict.renderer_meets_budget);
        assert!(!verdict.cadence_meets_budget);
        assert!(!verdict.input_meets_budget);
    }

    #[test]
    fn typing_budget_rejects_missing_input_latency_samples() {
        let verdict = analyze_metrics(TypingMetrics {
            display_period_ns: 8_333_333,
            current_fps: 120.0,
            work_p99_us: 2_000,
            interval_p99_us: 8_900,
            interval_max_us: 10_000,
            judder_ratio: 0.0,
            input_latency_p99_us: 5_000,
            input_latency_samples: 1,
        });

        assert!(!verdict.passed());
        assert!(!verdict.input_meets_budget);
    }
}
