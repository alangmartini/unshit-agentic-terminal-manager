use std::io;

use terminal_manager_diagnostics::{
    is_supported_protocol_version, AppIdentity, DiagnosticCapabilities, DiagnosticCommand,
    DiagnosticEventFamily, DiagnosticProtocolError, DiagnosticRequest, DiagnosticResponse,
    SnapshotOptions, DIAGNOSTIC_PROTOCOL_VERSION,
};

use super::config::DiagnosticConfig;
use super::snapshot;
use super::transport;
use crate::state::SharedState;

pub async fn run(config: DiagnosticConfig, shared: SharedState) -> io::Result<()> {
    transport::run(config, shared).await
}

pub struct DiagnosticAppContext {
    pub shared: SharedState,
    pub diagnostic_endpoint: Option<String>,
}

pub fn handle_request<F>(
    request: DiagnosticRequest,
    expected_token: &str,
    app_context: F,
) -> DiagnosticResponse
where
    F: FnOnce() -> DiagnosticAppContext,
{
    if request.token != expected_token {
        return protocol_error("unauthorized", "invalid diagnostic token", false);
    }

    match request.command {
        DiagnosticCommand::Hello {
            required_protocol_version,
        } => {
            if let Some(version) = required_protocol_version {
                if let Err(err) = is_supported_protocol_version(&version) {
                    return protocol_error("unsupported_protocol", &err.to_string(), false);
                }
            }

            DiagnosticResponse::Hello {
                protocol_version: DIAGNOSTIC_PROTOCOL_VERSION.to_owned(),
                enabled_features: enabled_features(),
                app: app_identity(),
                capabilities: handshake_capabilities(),
            }
        }
        DiagnosticCommand::Snapshot { reason, options } => handle_snapshot(app_context, reason, options),
        DiagnosticCommand::EvaluateInvariants { scope } => {
            let context = app_context();
            match snapshot::evaluate_invariants(&context.shared, scope) {
                Ok(results) => DiagnosticResponse::InvariantResults { results },
                Err(message) => protocol_error("invariant_evaluation_failed", &message, false),
            }
        }
        DiagnosticCommand::PrepareDeterministicMode { options } => DiagnosticResponse::Ack {
            message: Some(format!(
                "deterministic mode acknowledged as a no-op; requested disable_animations={}, disable_background_timers={}, fixed_clock_utc={}",
                options.disable_animations,
                options.disable_background_timers,
                options.fixed_clock_utc.as_deref().unwrap_or("none")
            )),
        },
        _ => protocol_error(
            "unsupported_command",
            "diagnostic command is not implemented in this slice",
            false,
        ),
    }
}

fn handle_snapshot(
    app_context: impl FnOnce() -> DiagnosticAppContext,
    reason: String,
    options: SnapshotOptions,
) -> DiagnosticResponse {
    if options.include_terminal_buffer {
        return protocol_error(
            "unsupported_snapshot_option",
            "terminal buffer contents are excluded from diagnostic snapshots",
            false,
        );
    }

    let context = app_context();
    match snapshot::collect_snapshot(&context.shared, reason, context.diagnostic_endpoint) {
        Ok(snapshot) => DiagnosticResponse::Snapshot { snapshot },
        Err(message) => protocol_error("snapshot_collection_failed", &message, false),
    }
}

fn protocol_error(code: &str, message: &str, retryable: bool) -> DiagnosticResponse {
    DiagnosticResponse::Error {
        error: DiagnosticProtocolError {
            code: code.to_owned(),
            message: message.to_owned(),
            retryable,
            details: serde_json::Value::Object(serde_json::Map::new()),
        },
    }
}

fn app_identity() -> AppIdentity {
    AppIdentity {
        name: "terminal-manager".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        build: build_identity(),
        commit: option_env!("GIT_COMMIT").map(str::to_owned),
        process_id: Some(std::process::id()),
    }
}

fn build_identity() -> String {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    format!("{profile}-{}", std::env::consts::ARCH)
}

fn enabled_features() -> Vec<String> {
    let mut features = vec!["diagnostics".to_owned()];
    if cfg!(feature = "input-latency-histogram") {
        features.push("input-latency-histogram".to_owned());
    }
    if cfg!(feature = "profiling") {
        features.push("profiling".to_owned());
    }
    features
}

fn handshake_capabilities() -> DiagnosticCapabilities {
    DiagnosticCapabilities {
        supported_protocol_versions: vec![DIAGNOSTIC_PROTOCOL_VERSION.to_owned()],
        transports: vec!["named_pipe".to_owned()],
        commands: vec![
            "hello".to_owned(),
            "snapshot".to_owned(),
            "evaluate_invariants".to_owned(),
            "prepare_deterministic_mode".to_owned(),
        ],
        event_families: Vec::<DiagnosticEventFamily>::new(),
        snapshots: true,
        invariants: true,
        step_markers: false,
        deterministic_mode: true,
        flush: false,
    }
}

pub(crate) fn invalid_request_response(message: &str) -> DiagnosticResponse {
    protocol_error("invalid_request", message, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello_request(token: &str) -> DiagnosticRequest {
        DiagnosticRequest {
            token: token.to_owned(),
            command: DiagnosticCommand::Hello {
                required_protocol_version: Some(DIAGNOSTIC_PROTOCOL_VERSION.to_owned()),
            },
            ..Default::default()
        }
    }

    #[test]
    fn hello_reports_protocol_pid_build_features_and_capabilities() {
        let response = handle_request(hello_request("secret"), "secret", || unreachable!());

        let DiagnosticResponse::Hello {
            protocol_version,
            enabled_features,
            app,
            capabilities,
        } = response
        else {
            panic!("expected hello response");
        };

        assert_eq!(protocol_version, DIAGNOSTIC_PROTOCOL_VERSION);
        assert_eq!(app.name, "terminal-manager");
        assert_eq!(app.process_id, Some(std::process::id()));
        assert!(!app.build.is_empty());
        assert!(enabled_features.contains(&"diagnostics".to_owned()));
        assert!(capabilities.commands.contains(&"hello".to_owned()));
        assert!(capabilities.commands.contains(&"snapshot".to_owned()));
        assert!(capabilities.snapshots);
        assert!(capabilities.invariants);
        assert_eq!(capabilities.transports, vec!["named_pipe"]);
    }

    #[test]
    fn missing_or_wrong_token_is_rejected_without_app_state() {
        for token in ["", "wrong"] {
            let response = handle_request(hello_request(token), "secret", || unreachable!());

            let DiagnosticResponse::Error { error } = response else {
                panic!("expected unauthorized response");
            };

            assert_eq!(error.code, "unauthorized");
            assert!(error.details.as_object().unwrap().is_empty());
            assert!(!error.message.contains("terminal-manager"));
            assert!(!error.message.contains(&std::process::id().to_string()));
        }
    }

    #[test]
    fn snapshot_and_invariant_commands_return_structured_results_after_token_validation() {
        let shared = std::sync::Arc::new(std::sync::Mutex::new(crate::state::seed_state()));

        let snapshot_response = handle_request(
            DiagnosticRequest {
                token: "secret".to_owned(),
                command: DiagnosticCommand::Snapshot {
                    reason: "protocol-test".to_owned(),
                    options: Default::default(),
                },
                ..Default::default()
            },
            "secret",
            || DiagnosticAppContext {
                shared: shared.clone(),
                diagnostic_endpoint: None,
            },
        );
        assert!(matches!(
            snapshot_response,
            DiagnosticResponse::Snapshot { .. }
        ));

        let invariant_response = handle_request(
            DiagnosticRequest {
                token: "secret".to_owned(),
                command: DiagnosticCommand::EvaluateInvariants {
                    scope: terminal_manager_diagnostics::InvariantScope::All,
                },
                ..Default::default()
            },
            "secret",
            || DiagnosticAppContext {
                shared,
                diagnostic_endpoint: None,
            },
        );
        let DiagnosticResponse::InvariantResults { results } = invariant_response else {
            panic!("expected invariant results");
        };
        assert!(results
            .iter()
            .any(|result| result.id == "app.active_pane.exists"));
    }

    #[test]
    fn snapshot_buffer_request_returns_protocol_error() {
        let response = handle_request(
            DiagnosticRequest {
                token: "secret".to_owned(),
                command: DiagnosticCommand::Snapshot {
                    reason: "debug".to_owned(),
                    options: SnapshotOptions {
                        include_terminal_buffer: true,
                    },
                },
                ..Default::default()
            },
            "secret",
            || unreachable!("buffer rejection must not touch app state"),
        );

        let DiagnosticResponse::Error { error } = response else {
            panic!("expected protocol error");
        };
        assert_eq!(error.code, "unsupported_snapshot_option");
    }

    #[test]
    fn deterministic_mode_ack_makes_noop_explicit() {
        let response = handle_request(
            DiagnosticRequest {
                token: "secret".to_owned(),
                command: DiagnosticCommand::PrepareDeterministicMode {
                    options: terminal_manager_diagnostics::DeterministicModeOptions {
                        disable_animations: true,
                        disable_background_timers: true,
                        fixed_clock_utc: None,
                    },
                },
                ..Default::default()
            },
            "secret",
            || unreachable!("deterministic no-op must not touch app state"),
        );

        let DiagnosticResponse::Ack { message } = response else {
            panic!("expected ack");
        };
        assert!(message.unwrap().contains("no-op"));
    }
}
