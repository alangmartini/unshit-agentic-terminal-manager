//! Cold-start stage timing.
//!
//! Startup is the one path where a regression is invisible in every other
//! signal: no frame is late, no PTY misbehaves, the app is simply slow to
//! appear. This module records when each bring-up stage completed, relative to
//! **process creation** (not `main` entry — the loader and static initializers
//! are part of what the user waits for), and emits one structured JSON line per
//! launch so a later session can attribute a slow start without reproducing it.
//!
//! Cost model: [`mark`] pushes a `(&'static str, u64)` into a pre-sized `Vec`
//! behind an uncontended mutex — tens of nanoseconds, and it happens at most a
//! couple dozen times per process. The file write happens once, on a detached
//! thread, after the first frame is already on screen, so no I/O lands on the
//! path being measured.
//!
//! Read the log with:
//! `Get-Content $env:APPDATA\com.godly.terminal\startup-events.jsonl -Tail 1`

use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// Env var that echoes the summary to stderr in addition to the log file.
/// Set by `scripts/startup-bench.ps1`.
const ENV_TRACE: &str = "TM_STARTUP_TRACE";

const LOG_FILE_NAME: &str = "startup-events.jsonl";

/// Expected stage count; sized so the recorder never reallocates mid-startup.
const EXPECTED_STAGES: usize = 24;

struct Recorder {
    /// Wall-clock reference for stage deltas.
    epoch: Instant,
    /// Microseconds the process had already been alive at `epoch`. Non-zero on
    /// Windows, where process creation time is observable; zero elsewhere.
    preamble_us: u64,
    stages: Mutex<Vec<(&'static str, u64)>>,
}

fn recorder() -> &'static Recorder {
    static RECORDER: OnceLock<Recorder> = OnceLock::new();
    RECORDER.get_or_init(|| Recorder {
        epoch: Instant::now(),
        preamble_us: process_age_us(),
        stages: Mutex::new(Vec::with_capacity(EXPECTED_STAGES)),
    })
}

/// Microseconds between process creation and now.
///
/// Windows reports process creation time directly, which captures image load,
/// DLL resolution, and static initialization — all of it time the user is
/// staring at nothing. Platforms without a cheap equivalent report 0 and
/// therefore measure from `main` entry instead; the emitted record says which
/// via `measures_process_load`.
#[cfg(windows)]
fn process_age_us() -> u64 {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    fn to_u64(ft: FILETIME) -> u64 {
        (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime)
    }

    let mut created = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut exited = created;
    let mut kernel = created;
    let mut user = created;
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    };
    if ok == 0 {
        return 0;
    }
    let mut now = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    unsafe { GetSystemTimeAsFileTime(&mut now) };
    // FILETIME ticks are 100ns. A clock that appears to run backwards (rare,
    // but possible across a time adjustment) reports 0 rather than a huge
    // wrapped value.
    to_u64(now).saturating_sub(to_u64(created)) / 10
}

#[cfg(not(windows))]
fn process_age_us() -> u64 {
    0
}

/// Start the clock. Call as the very first statement in `main` so the
/// `main_entry` stage measures loader cost honestly.
pub fn init() {
    let r = recorder();
    mark_at(r, "main_entry");
}

/// Record that `stage` just completed. `stage` must be a static string so the
/// record has bounded label cardinality.
pub fn mark(stage: &'static str) {
    mark_at(recorder(), stage);
}

fn mark_at(r: &Recorder, stage: &'static str) {
    let at_us = r.preamble_us + r.epoch.elapsed().as_micros() as u64;
    if let Ok(mut stages) = r.stages.lock() {
        stages.push((stage, at_us));
    }
}

/// Microseconds since process creation, for callers that want the raw number
/// (the FPS overlay, tests) rather than a recorded stage.
pub fn elapsed_us() -> u64 {
    let r = recorder();
    r.preamble_us + r.epoch.elapsed().as_micros() as u64
}

/// Emit the collected stage timings as one JSON line and reset the recorder so
/// a second call cannot double-report.
///
/// Safe to call from the frame callback: the formatting and the file append
/// both happen on a detached thread.
pub fn finish(context: Context) {
    let r = recorder();
    let stages = match r.stages.lock() {
        Ok(mut guard) if !guard.is_empty() => std::mem::take(&mut *guard),
        _ => return,
    };
    let trace = std::env::var_os(ENV_TRACE).is_some();
    std::thread::Builder::new()
        .name("startup-perf".into())
        .spawn(move || {
            let line = render_record(&stages, &context);
            if trace {
                eprintln!("{line}");
            }
            log::info!("{line}");
            append_line(&line);
        })
        .ok();
}

/// Cardinality-safe context recorded alongside the timings. No paths, no
/// workspace names, no error text — just the shape of the workload, which is
/// what makes one launch slower than another.
#[derive(Clone, Copy, Debug, Default)]
pub struct Context {
    pub workspaces: usize,
    pub panes: usize,
    /// Whether this launch had to spawn the ptyd daemon rather than attach to
    /// a running one. The single biggest source of run-to-run variance.
    pub daemon_spawned: bool,
    pub restored_layout: bool,
}

fn render_record(stages: &[(&'static str, u64)], context: &Context) -> String {
    let mut out = String::with_capacity(512);
    let total_us = stages.last().map(|(_, us)| *us).unwrap_or(0);
    out.push_str(r#"{"event":"app.startup","level":"info","correlation_id":""#);
    out.push_str(&format!("process-{}", std::process::id()));
    out.push_str(r#"","measures_process_load":"#);
    out.push_str(if cfg!(windows) { "true" } else { "false" });
    out.push_str(r#","workspaces":"#);
    out.push_str(&context.workspaces.to_string());
    out.push_str(r#","panes":"#);
    out.push_str(&context.panes.to_string());
    out.push_str(r#","daemon_spawned":"#);
    out.push_str(if context.daemon_spawned {
        "true"
    } else {
        "false"
    });
    out.push_str(r#","restored_layout":"#);
    out.push_str(if context.restored_layout {
        "true"
    } else {
        "false"
    });
    out.push_str(r#","total_ms":"#);
    push_ms(&mut out, total_us);
    out.push_str(r#","stages":["#);
    let mut previous_us = 0u64;
    for (i, (stage, at_us)) in stages.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(r#"{"stage":""#);
        out.push_str(stage);
        out.push_str(r#"","at_ms":"#);
        push_ms(&mut out, *at_us);
        out.push_str(r#","delta_ms":"#);
        push_ms(&mut out, at_us.saturating_sub(previous_us));
        out.push('}');
        previous_us = *at_us;
    }
    out.push_str("]}");
    out
}

fn push_ms(out: &mut String, us: u64) {
    out.push_str(&format!("{:.2}", us as f64 / 1000.0));
}

/// Append one line to the instance profile's startup log, bounding the file so
/// a long-lived install cannot grow it without limit.
fn append_line(line: &str) {
    use std::io::Write;

    let Some(path) = crate::profile::config_dir().map(|dir| dir.join(LOG_FILE_NAME)) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // One ~600 byte line per launch; 512 KiB is thousands of launches of
    // history. Truncate rather than rotate: startup history has no value once
    // it is that old, and a second file would be one more thing to find.
    const MAX_LOG_BYTES: u64 = 512 * 1024;
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(file, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_reports_absolute_and_per_stage_timings() {
        let stages = [
            ("main_entry", 1_500u64),
            ("daemon", 4_000),
            ("first_frame", 9_250),
        ];
        let record = render_record(
            &stages,
            &Context {
                workspaces: 7,
                panes: 10,
                daemon_spawned: true,
                restored_layout: true,
            },
        );

        assert!(record.contains(r#""event":"app.startup""#));
        assert!(record.contains(r#""workspaces":7"#));
        assert!(record.contains(r#""panes":10"#));
        assert!(record.contains(r#""daemon_spawned":true"#));
        assert!(record.contains(r#""total_ms":9.25"#));
        // Deltas attribute cost to the stage that spent it, which is the whole
        // point: `daemon` took 2.5ms, not the 4.0ms it finished at.
        assert!(record.contains(r#"{"stage":"daemon","at_ms":4.00,"delta_ms":2.50}"#));
        assert!(record.contains(r#"{"stage":"first_frame","at_ms":9.25,"delta_ms":5.25}"#));
    }

    #[test]
    fn record_is_valid_json() {
        let stages = [("main_entry", 10u64), ("first_frame", 20)];
        let record = render_record(&stages, &Context::default());
        let parsed: serde_json::Value =
            serde_json::from_str(&record).expect("startup record must be parseable JSON");
        assert_eq!(parsed["event"], "app.startup");
        assert_eq!(parsed["stages"].as_array().expect("stages array").len(), 2);
    }

    #[test]
    fn marks_are_monotonic_and_include_process_load_on_windows() {
        init();
        mark("test_stage_a");
        mark("test_stage_b");
        let r = recorder();
        let stages = r.stages.lock().expect("stage lock");
        let values: Vec<u64> = stages.iter().map(|(_, us)| *us).collect();
        assert!(
            values.windows(2).all(|w| w[1] >= w[0]),
            "stage timings must be monotonic: {values:?}"
        );
        #[cfg(windows)]
        assert!(
            values[0] > 0,
            "windows builds must measure from process creation, not main entry"
        );
    }
}
