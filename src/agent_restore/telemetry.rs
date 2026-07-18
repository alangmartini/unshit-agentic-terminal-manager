use std::io::Write;
use std::path::Path;

use serde::Serialize;

use super::AgentKind;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Copy, Debug)]
pub enum EventName {
    MetadataSaved,
    CandidateFound,
    Attached,
    ResumePending,
    ResumeSpawned,
    ResumeFailed,
    HookConfigChanged,
    HookObserved,
    HookRejected,
    CloseStatePersisted,
}

impl EventName {
    fn as_str(self) -> &'static str {
        match self {
            Self::MetadataSaved => "agent_restore.metadata_saved",
            Self::CandidateFound => "agent_restore.candidate_found",
            Self::Attached => "agent_restore.attached",
            Self::ResumePending => "agent_restore.resume_pending",
            Self::ResumeSpawned => "agent_restore.resume_spawned",
            Self::ResumeFailed => "agent_restore.resume_failed",
            Self::HookConfigChanged => "agent_restore.hook_config_changed",
            Self::HookObserved => "agent_restore.hook_observed",
            Self::HookRejected => "agent_restore.hook_rejected",
            Self::CloseStatePersisted => "agent_restore.close_state_persisted",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct RestoreEvent {
    pub timestamp_unix_ms: u64,
    pub correlation_id: String,
    pub event: &'static str,
    pub level: Level,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<AgentKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<&'static str>,
}

impl RestoreEvent {
    pub fn new(correlation_id: &str, event: EventName, level: Level) -> Self {
        Self {
            timestamp_unix_ms: super::now_unix_ms(),
            correlation_id: if super::is_valid_session_id(correlation_id) {
                correlation_id.to_string()
            } else {
                "unknown".to_string()
            },
            event: event.as_str(),
            level,
            provider: None,
            workspace_id: None,
            pane_id: None,
            source: None,
            outcome: None,
            error_kind: None,
        }
    }
}

pub fn record(event: &RestoreEvent) {
    let line = serde_json::to_string(event).unwrap_or_else(|_| {
        r#"{"event":"agent_restore.telemetry_failed","level":"error","correlation_id":"unknown"}"#
            .to_string()
    });
    match event.level {
        Level::Info => log::info!("{line}"),
        Level::Warn => log::warn!("{line}"),
        Level::Error => log::error!("{line}"),
    }
    if cfg!(test) {
        return;
    }
    let Some(path) = default_path() else {
        return;
    };
    if record_to(&path, event).is_err() {
        // The structured logger line above remains the fallback signal.
        // Do not include the raw IO error because paths and OS messages
        // can contain user data.
        log::warn!(
            "{{\"event\":\"agent_restore.telemetry_write_failed\",\"level\":\"warn\",\"correlation_id\":{:?}}}",
            event.correlation_id
        );
    }
}

pub fn default_path() -> Option<std::path::PathBuf> {
    crate::profile::config_dir().map(|directory| directory.join("agent-restore-events.jsonl"))
}

fn record_to(path: &Path, event: &RestoreEvent) -> std::io::Result<()> {
    const MAX_LOG_BYTES: u64 = 512 * 1024;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    #[cfg(unix)]
    if path.exists() {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
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
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    let mut line = serde_json::to_vec(event).map_err(std::io::Error::other)?;
    line.push(b'\n');
    file.write_all(&line)?;
    file.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_log(tag: &str) -> std::path::PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let count = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir()
            .join(format!(
                "tm-agent-telemetry-{tag}-{}-{count}",
                std::process::id()
            ))
            .join("agent-restore-events.jsonl")
    }

    #[test]
    fn writes_queryable_redacted_json_event() {
        let path = temp_log("event");
        let correlation = crate::agent_restore::generate_session_id();
        let conversation_id = "24c31fc8-8200-4773-8a0b-0447bd64bcdc";
        let cwd = r"C:\Users\someone\private-project";
        let mut event = RestoreEvent::new(&correlation, EventName::ResumeSpawned, Level::Info);
        event.provider = Some(AgentKind::Claude);
        event.workspace_id = Some(7);
        event.pane_id = Some(3);
        event.source = Some("startup");
        event.outcome = Some("spawned");
        record_to(&path, &event).expect("write telemetry");

        let body = std::fs::read_to_string(&path).expect("read telemetry");
        let value: serde_json::Value = serde_json::from_str(body.trim()).expect("json line");
        assert_eq!(value["event"], "agent_restore.resume_spawned");
        assert_eq!(value["correlation_id"], correlation);
        assert_eq!(value["provider"], "claude");
        assert_eq!(value["workspace_id"], 7);
        assert_eq!(value["pane_id"], 3);
        assert!(!body.contains(conversation_id));
        assert!(!body.contains(cwd));
        let _ = std::fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[cfg(unix)]
    #[test]
    fn telemetry_write_tightens_legacy_world_readable_mode() {
        use std::os::unix::fs::PermissionsExt;

        let path = temp_log("private-migration");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("parent");
        std::fs::write(&path, b"").expect("legacy log");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("legacy mode");
        let correlation = crate::agent_restore::generate_session_id();
        let event = RestoreEvent::new(&correlation, EventName::Attached, Level::Info);

        record_to(&path, &event).expect("telemetry write");

        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        let _ = std::fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn event_names_are_stable_and_namespaced() {
        assert_eq!(
            EventName::MetadataSaved.as_str(),
            "agent_restore.metadata_saved"
        );
        assert_eq!(
            EventName::HookRejected.as_str(),
            "agent_restore.hook_rejected"
        );
        assert_eq!(
            EventName::CloseStatePersisted.as_str(),
            "agent_restore.close_state_persisted"
        );
    }
}
