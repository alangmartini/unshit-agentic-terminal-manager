//! Bounded, rotating JSONL sink shared by per-feature lifecycle telemetry
//! (editor, Flow Explorer). One serialized record per line; when the file
//! reaches `max_bytes` it is renamed to `<stem>.jsonl.1`, replacing any
//! previous rotation, and a fresh file starts. Writes are synchronous, so
//! callers use it only on lifecycle transitions, never on the render or
//! keystroke path.

use std::io::Write;
use std::path::Path;

use serde::Serialize;

/// Rotation threshold every feature sink uses unless it has a reason not to.
pub const DEFAULT_MAX_LOG_BYTES: u64 = 512 * 1024;

pub fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

/// Append one record as a JSON line, rotating first when the file is
/// already at or over `max_bytes`.
pub fn append_rotating_jsonl<T: Serialize>(
    path: &Path,
    record: &T,
    max_bytes: u64,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if path
        .metadata()
        .map(|metadata| metadata.len() >= max_bytes)
        .unwrap_or(false)
    {
        let rotated = path.with_extension("jsonl.1");
        if rotated.exists() {
            std::fs::remove_file(&rotated)?;
        }
        std::fs::rename(path, rotated)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let mut line = serde_json::to_vec(record).map_err(std::io::Error::other)?;
    line.push(b'\n');
    file.write_all(&line)?;
    file.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[derive(Serialize)]
    struct Record<'a> {
        event: &'a str,
        n: u32,
    }

    fn unique_temp_path(tag: &str) -> std::path::PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "tm-telemetry-sink-{}-{}-{}",
            tag,
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        // PID reuse makes this name reachable again in a later run, and the
        // sink appends, so a stale leftover would corrupt the read.
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("events.jsonl")
    }

    #[test]
    fn appends_one_json_line_per_record() {
        let path = unique_temp_path("append");
        append_rotating_jsonl(&path, &Record { event: "a", n: 1 }, DEFAULT_MAX_LOG_BYTES).unwrap();
        append_rotating_jsonl(&path, &Record { event: "b", n: 2 }, DEFAULT_MAX_LOG_BYTES).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["event"], "a");
        assert_eq!(lines[1]["n"], 2);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn rotates_once_the_file_reaches_the_cap() {
        let path = unique_temp_path("rotate");
        let cap = 40;
        for n in 0..6 {
            append_rotating_jsonl(&path, &Record { event: "x", n }, cap).unwrap();
        }
        let rotated = path.with_extension("jsonl.1");
        assert!(rotated.exists(), "rotation file missing");
        assert!(path.metadata().unwrap().len() < cap * 2);
        // The live file always ends with the newest record.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.trim_end().ends_with("\"n\":5}"), "{text}");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
