use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sha2::{Digest, Sha256};
use terminal_manager_diagnostics::{
    AppIdentity, DeterministicModeOptions, DiagnosticCapabilities, DiagnosticCommand,
    DiagnosticEnvelope, DiagnosticEvent, DiagnosticEventFamily, DiagnosticRequest,
    DiagnosticResponse, InvariantEvaluation, InvariantScope, ObserveMode, SnapshotOptions,
    TerminalManagerSnapshot, DIAGNOSTIC_PROTOCOL_VERSION,
};

use crate::desktop_regression::artifacts::suite_artifact_name;

pub const ENV_DIAGNOSTICS_ENABLE: &str = "TM_DIAGNOSTICS_ENABLE";
pub const ENV_DIAGNOSTICS_PIPE_NAME: &str = "TM_DIAGNOSTICS_PIPE_NAME";
pub const ENV_DIAGNOSTICS_TOKEN: &str = "TM_DIAGNOSTICS_TOKEN";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticLaunchConfig {
    pub pipe_name: String,
    pub token: String,
}

impl DiagnosticLaunchConfig {
    pub fn pipe_path(&self) -> PathBuf {
        if self.pipe_name.starts_with(r"\\.\pipe\") {
            PathBuf::from(&self.pipe_name)
        } else {
            PathBuf::from(format!(r"\\.\pipe\{}", self.pipe_name))
        }
    }

    pub fn env_vars(&self) -> BTreeMap<&'static str, String> {
        BTreeMap::from([
            (ENV_DIAGNOSTICS_ENABLE, "1".to_owned()),
            (ENV_DIAGNOSTICS_PIPE_NAME, self.pipe_name.clone()),
            (ENV_DIAGNOSTICS_TOKEN, self.token.clone()),
        ])
    }
}

pub fn diagnostic_launch_for_mode(
    observe: ObserveMode,
    run_id: &str,
    suite_id: &str,
) -> Option<DiagnosticLaunchConfig> {
    if observe == ObserveMode::Off {
        return None;
    }

    let token = generate_token(run_id, suite_id);
    let token_prefix = &token[..12.min(token.len())];
    Some(DiagnosticLaunchConfig {
        pipe_name: format!(
            "tm-diagnostics-{}-{}-{token_prefix}",
            sanitize_pipe_component(run_id),
            sanitize_pipe_component(suite_id)
        ),
        token,
    })
}

fn generate_token(run_id: &str, suite_id: &str) -> String {
    let mut bytes = [0_u8; 32];
    if getrandom::fill(&mut bytes).is_err() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(run_id.as_bytes());
        hasher.update(suite_id.as_bytes());
        hasher.update(std::process::id().to_le_bytes());
        hasher.update(now.to_le_bytes());
        bytes.copy_from_slice(&hasher.finalize());
    }

    hex_encode(&bytes)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn sanitize_pipe_component(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticHello {
    pub protocol_version: String,
    pub enabled_features: Vec<String>,
    pub app: AppIdentity,
    pub capabilities: DiagnosticCapabilities,
}

#[derive(Debug, Clone)]
pub struct DiagnosticClient {
    pipe_path: PathBuf,
    token: String,
    connect_timeout: Duration,
}

impl DiagnosticClient {
    pub fn new(launch: &DiagnosticLaunchConfig) -> Self {
        Self {
            pipe_path: launch.pipe_path(),
            token: launch.token.clone(),
            connect_timeout: Duration::from_secs(5),
        }
    }

    pub fn wait_for_hello(&self, observe: ObserveMode) -> Result<DiagnosticHello, String> {
        let deadline = Instant::now() + self.connect_timeout;
        loop {
            match self.hello() {
                Ok(hello) => {
                    validate_capabilities(observe, &hello.capabilities)?;
                    return Ok(hello);
                }
                Err(_err) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(50));
                }
                Err(err) => return Err(err),
            }
        }
    }

    pub fn hello(&self) -> Result<DiagnosticHello, String> {
        response_into_hello(self.request(DiagnosticCommand::Hello {
            required_protocol_version: Some(DIAGNOSTIC_PROTOCOL_VERSION.to_owned()),
        })?)
    }

    pub fn prepare_deterministic_mode(&self) -> Result<(), String> {
        self.expect_ack(DiagnosticCommand::PrepareDeterministicMode {
            options: DeterministicModeOptions {
                disable_animations: true,
                disable_background_timers: true,
                fixed_clock_utc: None,
            },
        })
    }

    pub fn mark_step(&self, id: &str, label: &str) -> Result<(), String> {
        self.expect_ack(DiagnosticCommand::MarkStep {
            id: id.to_owned(),
            label: label.to_owned(),
        })
    }

    pub fn clear_step(&self, reason: &str) -> Result<(), String> {
        self.expect_ack(DiagnosticCommand::ClearStep {
            reason: Some(reason.to_owned()),
        })
    }

    pub fn snapshot(&self, reason: &str) -> Result<TerminalManagerSnapshot, String> {
        match self.request(DiagnosticCommand::Snapshot {
            reason: reason.to_owned(),
            options: SnapshotOptions::default(),
        })? {
            DiagnosticResponse::Snapshot { snapshot } => Ok(snapshot),
            response => Err(unexpected_response("snapshot", response)),
        }
    }

    pub fn evaluate_invariants(&self) -> Result<Vec<InvariantEvaluation>, String> {
        match self.request(DiagnosticCommand::EvaluateInvariants {
            scope: InvariantScope::All,
        })? {
            DiagnosticResponse::InvariantResults { results } => Ok(results),
            response => Err(unexpected_response("invariant results", response)),
        }
    }

    pub fn flush(&self) -> Result<(u64, u64), String> {
        match self.request(DiagnosticCommand::Flush)? {
            DiagnosticResponse::Flushed {
                events_flushed,
                dropped_events,
            } => Ok((events_flushed, dropped_events)),
            response => Err(unexpected_response("flush", response)),
        }
    }

    pub fn drain_events(&self) -> Result<(Vec<DiagnosticEnvelope<DiagnosticEvent>>, u64), String> {
        match self.request(DiagnosticCommand::DrainEvents { limit: None })? {
            DiagnosticResponse::Events {
                events,
                dropped_events,
            } => Ok((events, dropped_events)),
            response => Err(unexpected_response("events", response)),
        }
    }

    fn expect_ack(&self, command: DiagnosticCommand) -> Result<(), String> {
        match self.request(command)? {
            DiagnosticResponse::Ack { .. } => Ok(()),
            response => Err(unexpected_response("ack", response)),
        }
    }

    fn request(&self, command: DiagnosticCommand) -> Result<DiagnosticResponse, String> {
        let request = DiagnosticRequest {
            token: self.token.clone(),
            command,
            ..Default::default()
        };
        send_request(&self.pipe_path, &request)
    }
}

fn send_request(path: &Path, request: &DiagnosticRequest) -> Result<DiagnosticResponse, String> {
    let mut pipe = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| format!("failed to open diagnostic pipe {}: {e}", path.display()))?;
    let mut encoded =
        serde_json::to_vec(request).map_err(|e| format!("failed to encode request: {e}"))?;
    encoded.push(b'\n');
    pipe.write_all(&encoded)
        .map_err(|e| format!("failed to write diagnostic request: {e}"))?;
    pipe.flush()
        .map_err(|e| format!("failed to flush diagnostic request: {e}"))?;

    let mut reader = BufReader::new(pipe);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|e| format!("failed to read diagnostic response: {e}"))?;
    if line.trim().is_empty() {
        return Err("diagnostic response was empty".to_owned());
    }
    let response = serde_json::from_str::<DiagnosticResponse>(&line)
        .map_err(|e| format!("failed to decode diagnostic response: {e}"))?;
    match response {
        DiagnosticResponse::Error { error } => Err(format!(
            "diagnostic protocol error {}: {}",
            error.code, error.message
        )),
        other => Ok(other),
    }
}

pub(crate) fn response_into_hello(response: DiagnosticResponse) -> Result<DiagnosticHello, String> {
    match response {
        DiagnosticResponse::Hello {
            protocol_version,
            enabled_features,
            app,
            capabilities,
        } => {
            if protocol_version != DIAGNOSTIC_PROTOCOL_VERSION {
                return Err(format!(
                    "unsupported diagnostic protocol {protocol_version}; expected {DIAGNOSTIC_PROTOCOL_VERSION}"
                ));
            }
            Ok(DiagnosticHello {
                protocol_version,
                enabled_features,
                app,
                capabilities,
            })
        }
        DiagnosticResponse::Error { error } => Err(format!(
            "diagnostic protocol error {}: {}",
            error.code, error.message
        )),
        response => Err(unexpected_response("hello", response)),
    }
}

pub fn validate_capabilities(
    observe: ObserveMode,
    capabilities: &DiagnosticCapabilities,
) -> Result<(), String> {
    if observe == ObserveMode::Off {
        return Ok(());
    }

    let mut missing = Vec::new();
    if !capabilities
        .supported_protocol_versions
        .iter()
        .any(|version| version == DIAGNOSTIC_PROTOCOL_VERSION)
    {
        missing.push(format!("protocol {DIAGNOSTIC_PROTOCOL_VERSION}"));
    }
    if !capabilities
        .transports
        .iter()
        .any(|transport| transport == "named_pipe")
    {
        missing.push("named_pipe transport".to_owned());
    }
    require_command(capabilities, "hello", &mut missing);
    require_command(capabilities, "snapshot", &mut missing);
    require_command(capabilities, "flush", &mut missing);
    require_command(capabilities, "drain_events", &mut missing);
    if !capabilities.snapshots {
        missing.push("snapshots".to_owned());
    }
    if !capabilities.flush {
        missing.push("flush capability".to_owned());
    }
    require_event_family(capabilities, DiagnosticEventFamily::Log, &mut missing);

    if observe == ObserveMode::Full {
        require_command(capabilities, "mark_step", &mut missing);
        require_command(capabilities, "clear_step", &mut missing);
        require_command(capabilities, "evaluate_invariants", &mut missing);
        require_command(capabilities, "prepare_deterministic_mode", &mut missing);
        require_event_family(capabilities, DiagnosticEventFamily::TestStep, &mut missing);
        require_event_family(capabilities, DiagnosticEventFamily::Invariant, &mut missing);
        if !capabilities.step_markers {
            missing.push("step_markers".to_owned());
        }
        if !capabilities.invariants {
            missing.push("invariants".to_owned());
        }
        if !capabilities.deterministic_mode {
            missing.push("deterministic_mode".to_owned());
        }
    }

    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "diagnostic capabilities missing required {} support: {}",
            observe_label(observe),
            missing.join(", ")
        ))
    }
}

fn require_command(
    capabilities: &DiagnosticCapabilities,
    command: &str,
    missing: &mut Vec<String>,
) {
    if !capabilities.commands.iter().any(|value| value == command) {
        missing.push(format!("{command} command"));
    }
}

fn require_event_family(
    capabilities: &DiagnosticCapabilities,
    family: DiagnosticEventFamily,
    missing: &mut Vec<String>,
) {
    if !capabilities.event_families.contains(&family) {
        missing.push(format!("{} event family", event_family_label(family)));
    }
}

fn observe_label(observe: ObserveMode) -> &'static str {
    match observe {
        ObserveMode::Off => "off",
        ObserveMode::Basic => "basic",
        ObserveMode::Full => "full",
    }
}

fn event_family_label(family: DiagnosticEventFamily) -> &'static str {
    match family {
        DiagnosticEventFamily::TestStep => "test.step",
        DiagnosticEventFamily::Window => "window",
        DiagnosticEventFamily::Layout => "layout",
        DiagnosticEventFamily::Render => "render",
        DiagnosticEventFamily::Terminal => "terminal",
        DiagnosticEventFamily::Pty => "pty",
        DiagnosticEventFamily::Input => "input",
        DiagnosticEventFamily::State => "state",
        DiagnosticEventFamily::Invariant => "invariant",
        DiagnosticEventFamily::Log => "log",
    }
}

fn unexpected_response(expected: &str, response: DiagnosticResponse) -> String {
    format!("expected diagnostic {expected} response, received {response:?}")
}

pub fn write_json_artifact<T: Serialize>(
    run_dir: &Path,
    suite_id: &str,
    name: &str,
    value: &T,
) -> Result<String, String> {
    let artifact_name = suite_artifact_name(suite_id, name, "json");
    let path = run_dir.join(&artifact_name);
    let json = serde_json::to_string_pretty(value)
        .map_err(|e| format!("failed to serialize {artifact_name}: {e}"))?;
    std::fs::write(&path, json).map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    Ok(artifact_name)
}

pub fn write_diagnostic_events(
    run_dir: &Path,
    suite_id: &str,
    events: &[DiagnosticEnvelope<DiagnosticEvent>],
) -> Result<Vec<String>, String> {
    let events_name = suite_artifact_name(suite_id, "app.events", "jsonl");
    let logs_name = suite_artifact_name(suite_id, "app.logs", "jsonl");
    let mut events_body = String::new();
    let mut logs_body = String::new();

    for event in events {
        let line = serde_json::to_string(event)
            .map_err(|e| format!("failed to serialize diagnostic event: {e}"))?;
        events_body.push_str(&line);
        events_body.push('\n');
        if event.payload.family == DiagnosticEventFamily::Log {
            logs_body.push_str(&line);
            logs_body.push('\n');
        }
    }

    std::fs::write(run_dir.join(&events_name), events_body)
        .map_err(|e| format!("failed to write {events_name}: {e}"))?;
    std::fs::write(run_dir.join(&logs_name), logs_body)
        .map_err(|e| format!("failed to write {logs_name}: {e}"))?;
    Ok(vec![events_name, logs_name])
}

#[cfg(test)]
mod tests {
    use super::*;
    use terminal_manager_diagnostics::{
        DiagnosticCapabilities, DiagnosticEventFamily, DiagnosticResponse, ObserveMode,
        DIAGNOSTIC_PROTOCOL_VERSION,
    };

    #[test]
    fn off_mode_has_no_diagnostic_launch_environment() {
        assert!(diagnostic_launch_for_mode(ObserveMode::Off, "run-1", "edge").is_none());
    }

    #[test]
    fn observed_modes_set_required_launch_environment() {
        let launch = diagnostic_launch_for_mode(ObserveMode::Full, "run-1", "edge")
            .expect("full observe enables diagnostics");
        let env = launch.env_vars();

        assert_eq!(
            env.get(ENV_DIAGNOSTICS_ENABLE).map(String::as_str),
            Some("1")
        );
        assert_eq!(
            env.get(ENV_DIAGNOSTICS_PIPE_NAME).map(String::as_str),
            Some(launch.pipe_name.as_str())
        );
        assert_eq!(
            env.get(ENV_DIAGNOSTICS_TOKEN).map(String::as_str),
            Some(launch.token.as_str())
        );
        assert!(launch.pipe_name.contains("run-1"));
        assert!(launch.pipe_name.contains("edge"));
        assert!(launch.token.len() >= 32);
    }

    #[test]
    fn basic_capability_check_requires_protocol_transport_and_core_commands() {
        let mut capabilities = DiagnosticCapabilities::default();
        capabilities
            .commands
            .retain(|command| command != "snapshot");

        let err = validate_capabilities(ObserveMode::Basic, &capabilities).unwrap_err();

        assert!(err.contains("snapshot"));
    }

    #[test]
    fn full_capability_check_requires_steps_invariants_and_event_families() {
        let mut capabilities = DiagnosticCapabilities::default();
        capabilities.step_markers = false;
        capabilities
            .event_families
            .retain(|family| *family != DiagnosticEventFamily::Invariant);

        let err = validate_capabilities(ObserveMode::Full, &capabilities).unwrap_err();

        assert!(err.contains("step_markers"));
        assert!(err.contains("invariant event family"));
    }

    #[test]
    fn protocol_errors_are_returned_as_client_errors() {
        let response = DiagnosticResponse::Error {
            error: terminal_manager_diagnostics::DiagnosticProtocolError {
                code: "unauthorized".to_owned(),
                message: "invalid diagnostic token".to_owned(),
                retryable: false,
                details: serde_json::Value::Object(serde_json::Map::new()),
            },
        };

        let err = response_into_hello(response).unwrap_err();

        assert!(err.contains("unauthorized"));
        assert!(err.contains("invalid diagnostic token"));
    }

    #[test]
    fn hello_response_validates_protocol_version() {
        let response = DiagnosticResponse::Hello {
            protocol_version: "terminal-manager.diagnostics/v0".to_owned(),
            enabled_features: Vec::new(),
            app: Default::default(),
            capabilities: DiagnosticCapabilities::default(),
        };

        let err = response_into_hello(response).unwrap_err();

        assert!(err.contains(DIAGNOSTIC_PROTOCOL_VERSION));
    }
}
