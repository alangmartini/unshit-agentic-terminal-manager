//! Pending agent launches: once a second, look for the JSON each launch
//! was asked to write, and hand a finished file to the app state.
//!
//! There is no file watcher. One thread polls `metadata()` on every
//! pending output path. A file that exists, is older than the write
//! grace and ingests cleanly opens a pane; a file that fails to ingest
//! is reported once and forgotten (a bad file will not fix itself); a
//! launch whose pane has closed or whose deadline has passed is dropped.
//! Nothing here runs on the render path: the disk work happens on this
//! thread and only the bookkeeping takes the state lock.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, UNIX_EPOCH};

use unshit::app::{EventSink, ExternalEvent};

use super::model::FlowMode;
use super::pane::FlowPane;
use crate::state::{MutexExt, SharedState};

/// How often the pending outputs are checked.
pub const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// A launch that has not produced a file after this long is forgotten.
pub const PENDING_TIMEOUT_MS: u64 = 60 * 60 * 1000;

/// A file modified more recently than this is still being written. The
/// skill asks the agent to write `<path>.tmp` and rename, so this is a
/// second line of defence for producers that write in place.
pub const WRITE_GRACE_MS: u64 = 1_500;

/// One app-launched producer whose output has not arrived yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingFlow {
    pub flow_id: String,
    pub output_path: PathBuf,
    pub mode: FlowMode,
    /// `claude` or `codex`: a telemetry label, never a path or a prompt.
    pub agent: &'static str,
    pub started_unix_ms: u64,
}

/// What one poll of one pending launch found.
#[derive(Debug)]
pub enum PollOutcome {
    /// No file yet, or one that is still being written.
    Pending,
    /// The file ingested; the pane is ready to add.
    Ready(Box<FlowPane>),
    /// The file exists but is not a usable flow.
    Failed {
        reason: &'static str,
        message: String,
        os_error: Option<i32>,
    },
    /// The deadline passed without a usable file.
    TimedOut,
}

impl PollOutcome {
    pub fn is_pending(&self) -> bool {
        matches!(self, PollOutcome::Pending)
    }
}

/// Pure check of one pending launch at `now_unix_ms`.
pub fn poll_once(pending: &PendingFlow, now_unix_ms: u64) -> PollOutcome {
    if now_unix_ms.saturating_sub(pending.started_unix_ms) >= PENDING_TIMEOUT_MS {
        return PollOutcome::TimedOut;
    }
    let Ok(metadata) = std::fs::metadata(&pending.output_path) else {
        return PollOutcome::Pending;
    };
    if !metadata.is_file() {
        return PollOutcome::Pending;
    }
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|age| age.as_millis().min(u64::MAX as u128) as u64);
    if let Some(modified_ms) = modified_ms {
        // A modification time in the future (clock skew) counts as
        // settled rather than waiting until the deadline.
        if modified_ms <= now_unix_ms && now_unix_ms - modified_ms < WRITE_GRACE_MS {
            return PollOutcome::Pending;
        }
    }
    match FlowPane::open(&pending.output_path) {
        Ok(pane) => PollOutcome::Ready(Box::new(pane)),
        Err(err) => PollOutcome::Failed {
            reason: err.reason(),
            message: err.message(&pending.output_path),
            os_error: err.os_error(),
        },
    }
}

/// Start the poll thread. Returns immediately. Safe before the event
/// loop exists: `sink` may still be empty, in which case a rebuild is
/// simply not requested and the next poll requests one.
pub fn start(shared: SharedState, sink: Arc<OnceLock<EventSink>>) {
    let spawned = std::thread::Builder::new()
        .name("flow-explorer-poll".into())
        .spawn(move || loop {
            std::thread::sleep(POLL_INTERVAL);
            let snapshot = {
                let guard = shared.lock_recover();
                crate::state::flow_poll_snapshot(&guard)
            };
            if snapshot.is_empty() {
                continue;
            }
            let now = crate::agent_restore::now_unix_ms();
            let outcomes: Vec<(u32, PollOutcome)> = snapshot
                .iter()
                .map(|(pane_id, pending)| (*pane_id, poll_once(pending, now)))
                .filter(|(_, outcome)| !outcome.is_pending())
                .collect();
            if outcomes.is_empty() {
                continue;
            }
            let changed = {
                let mut guard = shared.lock_recover();
                crate::state::apply_flow_poll(&mut guard, outcomes, now)
            };
            if changed {
                if let Some(sink) = sink.get() {
                    let _ = sink.send(ExternalEvent::RequestRebuild);
                }
            }
        });
    if let Err(err) = spawned {
        log::warn!(
            "{{\"event\":\"flow.poller_spawn_failed\",\"level\":\"warn\",\"error\":{:?}}}",
            err.to_string()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_explorer::test_support::fixture_path;
    use std::path::Path;

    /// Comfortably past the write grace, added to a real "now".
    const SETTLED_MS: u64 = 10 * WRITE_GRACE_MS;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tm-flow-poll-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    fn pending_in(dir: &Path, started_unix_ms: u64) -> PendingFlow {
        PendingFlow {
            flow_id: "explain-1".into(),
            output_path: dir.join("explain-1.json"),
            mode: FlowMode::Explain,
            agent: "claude",
            started_unix_ms,
        }
    }

    fn now() -> u64 {
        crate::agent_restore::now_unix_ms()
    }

    #[test]
    fn missing_file_is_pending_until_the_deadline() {
        let dir = scratch("missing");
        let started = now();
        let pending = pending_in(&dir, started);
        assert!(poll_once(&pending, started).is_pending());
        assert!(poll_once(&pending, started + PENDING_TIMEOUT_MS - 1).is_pending());
        assert!(matches!(
            poll_once(&pending, started + PENDING_TIMEOUT_MS),
            PollOutcome::TimedOut
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finished_fixture_is_ready_once_the_write_grace_passed() {
        let dir = scratch("ready");
        let started = now();
        let pending = pending_in(&dir, started);
        // `fs::write` (not `fs::copy`) so the mtime is "now", not the fixture's.
        std::fs::write(
            &pending.output_path,
            std::fs::read(fixture_path()).expect("fixture"),
        )
        .expect("write");
        assert!(
            poll_once(&pending, now()).is_pending(),
            "a file younger than the write grace is not read yet"
        );
        match poll_once(&pending, now() + SETTLED_MS) {
            PollOutcome::Ready(pane) => {
                assert_eq!(pane.flow.nodes.len(), 11);
                assert_eq!(pane.flow_id, "explain-1");
            }
            other => panic!("expected Ready, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn garbage_fails_with_invalid_json_and_a_message() {
        let dir = scratch("garbage");
        let pending = pending_in(&dir, now());
        std::fs::write(&pending.output_path, b"{ not json").expect("write");
        match poll_once(&pending, now() + SETTLED_MS) {
            PollOutcome::Failed {
                reason,
                message,
                os_error,
            } => {
                assert_eq!(reason, "invalid_json");
                assert!(message.contains("explain-1.json"), "{message}");
                assert!(os_error.is_none());
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn producer_error_envelope_fails_with_its_reason() {
        let dir = scratch("envelope");
        let pending = pending_in(&dir, now());
        std::fs::write(
            &pending.output_path,
            br#"{"schema_version":1,"title":"x","repo_root":".","error":"no entry point matched",
                "processes":[],"nodes":[],"edges":[],"entries":[]}"#,
        )
        .expect("write");
        match poll_once(&pending, now() + SETTLED_MS) {
            PollOutcome::Failed {
                reason, message, ..
            } => {
                assert_eq!(reason, "producer_error");
                assert!(message.contains("no entry point matched"), "{message}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_directory_at_the_output_path_is_pending() {
        let dir = scratch("dir");
        let pending = pending_in(&dir, now());
        std::fs::create_dir_all(&pending.output_path).expect("dir");
        assert!(poll_once(&pending, now() + SETTLED_MS).is_pending());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
