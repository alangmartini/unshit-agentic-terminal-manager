use std::fs::{create_dir_all, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

fn terminal_trace_file_path_internal() -> PathBuf {
    std::env::var_os("TM_TRACE_TERMINAL_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".omx/logs/terminal-trace.log"))
}

pub fn terminal_trace_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("TM_TRACE_TERMINAL").is_some())
}

pub fn terminal_trace_file_path() -> PathBuf {
    terminal_trace_file_path_internal()
}

fn terminal_trace_file() -> Option<&'static Mutex<File>> {
    static FILE: OnceLock<Option<Mutex<File>>> = OnceLock::new();
    FILE.get_or_init(|| {
        if !terminal_trace_enabled() {
            return None;
        }

        let path = terminal_trace_file_path_internal();
        if let Some(parent) = path.parent() {
            let _ = create_dir_all(parent);
        }

        let file = OpenOptions::new().create(true).write(true).truncate(true).open(path).ok()?;

        Some(Mutex::new(file))
    })
    .as_ref()
}

pub fn append_terminal_trace_line(line: &str) {
    if !terminal_trace_enabled() {
        return;
    }

    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);

    if let Some(file) = terminal_trace_file() {
        if let Ok(mut guard) = file.lock() {
            let _ = writeln!(guard, "[{}] {}", timestamp_ms, line);
            let _ = guard.flush();
        }
    }
}
