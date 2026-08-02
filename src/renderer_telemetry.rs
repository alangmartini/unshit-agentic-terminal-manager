use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use unshit::app::{FrameMetrics, GlyphAtlasRecoveryEvent};

const MAX_LOG_BYTES: u64 = 512 * 1024;
const SLOW_FRAME_BUDGET_US: u64 = 8_333;
const SLOW_FRAME_SAMPLE_INTERVAL_MS: u64 = 250;
const TELEMETRY_QUEUE_CAPACITY: usize = 64;

static TELEMETRY_SENDER: OnceLock<Option<SyncSender<SlowFrameRecord>>> = OnceLock::new();
static FILE_WRITE_LOCK: Mutex<()> = Mutex::new(());
static LAST_SLOW_FRAME_SAMPLE_MS: AtomicU64 = AtomicU64::new(0);
static SLOW_FRAME_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static QUEUE_WARNING_ACTIVE: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Serialize)]
struct RendererRecoveryRecord {
    timestamp_unix_ms: u64,
    event: &'static str,
    level: &'static str,
    correlation_id: String,
    atlas_size: u32,
    resident_glyphs: u32,
    requested_width: u32,
    requested_height: u32,
    generation_before: u64,
    generation_after: u64,
    retry_succeeded: bool,
}

#[derive(Debug, Serialize)]
struct SlowFrameRecord {
    timestamp_unix_ms: u64,
    event: &'static str,
    level: &'static str,
    correlation_id: String,
    sample_sequence: u64,
    sample_interval_ms: u64,
    budget_us: u64,
    total_us: u64,
    tree_build_us: u64,
    style_resolve_us: u64,
    style_resolve_scope: &'static str,
    scale_us: u64,
    layout_us: u64,
    batch_build_us: u64,
    gpu_render_us: u64,
    present_wait_us: u64,
    present_hold_us: u64,
    present_call_us: u64,
    pacing_interval_us: u64,
    pacing_submit_us: u64,
    display_period_ns: u64,
    node_count: usize,
    quad_count: u32,
    glyph_count: u32,
}

impl SlowFrameRecord {
    fn from_metrics(metrics: &FrameMetrics, timestamp_unix_ms: u64) -> Self {
        Self {
            timestamp_unix_ms,
            event: "renderer.slow_frame",
            level: "warn",
            correlation_id: process_correlation_id().to_string(),
            sample_sequence: SLOW_FRAME_SEQUENCE.fetch_add(1, Ordering::Relaxed),
            sample_interval_ms: SLOW_FRAME_SAMPLE_INTERVAL_MS,
            budget_us: SLOW_FRAME_BUDGET_US,
            total_us: metrics.total_us,
            tree_build_us: metrics.tree_build_us,
            style_resolve_us: metrics.style_resolve_us,
            style_resolve_scope: metrics.style_resolve_scope.as_str(),
            scale_us: metrics.scale_us,
            layout_us: metrics.layout_us,
            batch_build_us: metrics.batch_build_us,
            gpu_render_us: metrics.gpu_render_us,
            present_wait_us: metrics.present_wait_us,
            present_hold_us: metrics.present_hold_us,
            present_call_us: metrics.present_call_us,
            pacing_interval_us: metrics.pacing_interval_us,
            pacing_submit_us: metrics.pacing_submit_us,
            display_period_ns: metrics.display_period_ns,
            node_count: metrics.node_count,
            quad_count: metrics.quad_count,
            glyph_count: metrics.glyph_count,
        }
    }
}

/// Start the bounded renderer telemetry writer before entering the render loop.
pub fn initialize() {
    let _ = telemetry_sender();
}

/// Persist content-free renderer recovery telemetry immediately. Recovery is
/// exceptional rather than a hot path, and retaining synchronous durability
/// ensures the event lands even if the process exits during a failed retry.
pub fn record_glyph_atlas_recovery(event: &GlyphAtlasRecoveryEvent) {
    let record = RendererRecoveryRecord {
        timestamp_unix_ms: now_unix_ms(),
        event: "renderer.glyph_atlas_recovery",
        level: if event.retry_succeeded {
            "warn"
        } else {
            "error"
        },
        correlation_id: event.correlation_id.clone(),
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
            "{{\"event\":\"renderer.telemetry_write_failed\",\"level\":\"warn\",\"correlation_id\":{:?},\"source_event\":{:?}}}",
            event.correlation_id,
            record.event,
        );
    }
}

/// Sample over-budget frames into durable stage telemetry. The callback does
/// no file I/O: a bounded channel moves serialization and rotation to a worker.
pub fn record_slow_frame(metrics: &FrameMetrics) {
    if metrics.total_us <= SLOW_FRAME_BUDGET_US {
        return;
    }

    let timestamp_unix_ms = now_unix_ms();
    let previous = LAST_SLOW_FRAME_SAMPLE_MS.load(Ordering::Relaxed);
    if timestamp_unix_ms.saturating_sub(previous) < SLOW_FRAME_SAMPLE_INTERVAL_MS
        || LAST_SLOW_FRAME_SAMPLE_MS
            .compare_exchange(
                previous,
                timestamp_unix_ms,
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .is_err()
    {
        return;
    }

    enqueue(SlowFrameRecord::from_metrics(metrics, timestamp_unix_ms));
}

pub fn default_path() -> Option<std::path::PathBuf> {
    crate::profile::config_dir().map(|directory| directory.join("renderer-events.jsonl"))
}

fn process_correlation_id() -> &'static str {
    static CORRELATION_ID: OnceLock<String> = OnceLock::new();
    CORRELATION_ID
        .get_or_init(|| format!("process-{}", std::process::id()))
        .as_str()
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn telemetry_sender() -> Option<&'static SyncSender<SlowFrameRecord>> {
    TELEMETRY_SENDER
        .get_or_init(|| {
            let correlation_id = process_correlation_id();
            let Some(path) = default_path() else {
                log::warn!(
                    "{{\"event\":\"renderer.telemetry_start_failed\",\"level\":\"warn\",\"correlation_id\":{correlation_id:?},\"reason\":\"config_directory_unavailable\"}}"
                );
                return None;
            };
            match spawn_writer(path) {
                Ok((sender, _worker)) => Some(sender),
                Err(error) => {
                    log::warn!(
                        "{{\"event\":\"renderer.telemetry_start_failed\",\"level\":\"warn\",\"correlation_id\":{correlation_id:?},\"reason\":\"thread_spawn_failed\",\"os_error_code\":{:?}}}",
                        error.raw_os_error(),
                    );
                    None
                }
            }
        })
        .as_ref()
}

fn enqueue(record: SlowFrameRecord) {
    let Some(sender) = telemetry_sender() else {
        return;
    };
    if let Err(error) = sender.try_send(record) {
        let (record, reason) = match error {
            TrySendError::Full(record) => (record, "queue_full"),
            TrySendError::Disconnected(record) => (record, "writer_disconnected"),
        };
        if !QUEUE_WARNING_ACTIVE.swap(true, Ordering::Relaxed) {
            log::warn!(
                "{{\"event\":\"renderer.telemetry_queue_unavailable\",\"level\":\"warn\",\"correlation_id\":{:?},\"source_event\":{:?},\"reason\":{reason:?}}}",
                record.correlation_id,
                record.event,
            );
        }
    }
}

fn spawn_writer(
    path: PathBuf,
) -> std::io::Result<(SyncSender<SlowFrameRecord>, std::thread::JoinHandle<()>)> {
    let (sender, receiver) = mpsc::sync_channel::<SlowFrameRecord>(TELEMETRY_QUEUE_CAPACITY);
    let worker = std::thread::Builder::new()
        .name("renderer-telemetry".to_string())
        .spawn(move || {
            while let Ok(record) = receiver.recv() {
                QUEUE_WARNING_ACTIVE.store(false, Ordering::Relaxed);
                if record_to(&path, &record).is_err() {
                    log::warn!(
                        "{{\"event\":\"renderer.telemetry_write_failed\",\"level\":\"warn\",\"correlation_id\":{:?},\"source_event\":{:?}}}",
                        record.correlation_id,
                        record.event,
                    );
                }
            }
        })?;
    Ok((sender, worker))
}

fn record_to<T: Serialize>(path: &Path, event: &T) -> std::io::Result<()> {
    let _guard = FILE_WRITE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
            correlation_id: "glyph-atlas-test".to_string(),
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

    #[test]
    fn slow_frame_event_is_persisted_by_worker_with_queryable_stage_scope() {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir()
            .join(format!(
                "tm-renderer-slow-frame-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ))
            .join("renderer-events.jsonl");
        let metrics = FrameMetrics {
            total_us: 73_230,
            style_resolve_us: 63_830,
            style_resolve_scope: unshit::app::StyleResolveScope::Document,
            layout_us: 3_960,
            batch_build_us: 4_550,
            node_count: 240,
            display_period_ns: 8_333_333,
            ..FrameMetrics::default()
        };
        let record = SlowFrameRecord::from_metrics(&metrics, 123);
        let (sender, worker) = spawn_writer(path.clone()).expect("spawn telemetry writer");

        sender.send(record).expect("enqueue slow frame");
        drop(sender);
        worker.join().expect("join telemetry writer");

        let body = std::fs::read_to_string(&path).expect("read telemetry");
        let value: serde_json::Value =
            serde_json::from_str(body.trim()).expect("valid JSONL record");
        assert_eq!(value["event"], "renderer.slow_frame");
        assert_eq!(value["style_resolve_scope"], "document");
        assert_eq!(value["style_resolve_us"], 63_830);
        assert_eq!(value["node_count"], 240);
        assert!(value["correlation_id"]
            .as_str()
            .is_some_and(|id| id.starts_with("process-")));
        assert!(value.get("text").is_none());
        assert!(value.get("terminal_output").is_none());
    }
}
