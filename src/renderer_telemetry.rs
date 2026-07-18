use std::io::Write;
use std::path::Path;

use serde::Serialize;
use unshit::app::GlyphAtlasRecoveryEvent;

#[derive(Debug, Serialize)]
struct RendererRecoveryRecord<'a> {
    timestamp_unix_ms: u64,
    event: &'static str,
    level: &'static str,
    correlation_id: &'a str,
    atlas_size: u32,
    resident_glyphs: u32,
    requested_width: u32,
    requested_height: u32,
    generation_before: u64,
    generation_after: u64,
    retry_succeeded: bool,
}

/// Persist content-free renderer recovery telemetry to a bounded JSONL file.
pub fn record_glyph_atlas_recovery(event: &GlyphAtlasRecoveryEvent) {
    let record = RendererRecoveryRecord {
        timestamp_unix_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64,
        event: "renderer.glyph_atlas_recovery",
        level: if event.retry_succeeded {
            "warn"
        } else {
            "error"
        },
        correlation_id: &event.correlation_id,
        atlas_size: event.atlas_size,
        resident_glyphs: event.resident_glyphs,
        requested_width: event.requested_width,
        requested_height: event.requested_height,
        generation_before: event.generation_before,
        generation_after: event.generation_after,
        retry_succeeded: event.retry_succeeded,
    };
    let Some(path) = default_path() else {
        return;
    };
    if record_to(&path, &record).is_err() {
        log::warn!(
            "{{\"event\":\"renderer.telemetry_write_failed\",\"level\":\"warn\",\"correlation_id\":{:?}}}",
            event.correlation_id
        );
    }
}

pub fn default_path() -> Option<std::path::PathBuf> {
    crate::profile::config_dir().map(|directory| directory.join("renderer-events.jsonl"))
}

fn record_to(path: &Path, event: &RendererRecoveryRecord<'_>) -> std::io::Result<()> {
    const MAX_LOG_BYTES: u64 = 512 * 1024;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if path
        .metadata()
        .map(|metadata| metadata.len() >= MAX_LOG_BYTES)
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
    let mut line = serde_json::to_vec(event).map_err(std::io::Error::other)?;
    line.push(b'\n');
    file.write_all(&line)?;
    file.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn renderer_recovery_event_is_queryable_and_contains_no_terminal_content() {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir()
            .join(format!(
                "tm-renderer-telemetry-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ))
            .join("renderer-events.jsonl");
        let record = RendererRecoveryRecord {
            timestamp_unix_ms: 123,
            event: "renderer.glyph_atlas_recovery",
            level: "warn",
            correlation_id: "glyph-atlas-test",
            atlas_size: 2048,
            resident_glyphs: 900,
            requested_width: 12,
            requested_height: 18,
            generation_before: 4,
            generation_after: 5,
            retry_succeeded: true,
        };

        record_to(&path, &record).expect("write renderer telemetry");
        let value: serde_json::Value = serde_json::from_str(
            std::fs::read_to_string(&path)
                .expect("read telemetry")
                .trim(),
        )
        .expect("valid JSONL record");
        assert_eq!(value["event"], "renderer.glyph_atlas_recovery");
        assert_eq!(value["correlation_id"], "glyph-atlas-test");
        assert_eq!(value["retry_succeeded"], true);
        assert!(value.get("text").is_none());
    }
}
