use std::io;

use terminal_manager_diagnostics::{
    is_supported_protocol_version, AppIdentity, DiagnosticCapabilities, DiagnosticCommand,
    DiagnosticEventFamily, DiagnosticProtocolError, DiagnosticRequest, DiagnosticResponse,
    InvariantOutcome, SnapshotOptions, DIAGNOSTIC_PROTOCOL_VERSION,
};

use super::config::DiagnosticConfig;
use super::events::DiagnosticEventStore;
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
    events: &DiagnosticEventStore,
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
        DiagnosticCommand::MarkStep { id, label } => {
            events.mark_step(id.clone(), label);
            DiagnosticResponse::Ack {
                message: Some(format!("marked diagnostic step {id}")),
            }
        }
        DiagnosticCommand::ClearStep { reason } => {
            events.clear_step(reason);
            DiagnosticResponse::Ack {
                message: Some("cleared diagnostic step".to_owned()),
            }
        }
        DiagnosticCommand::Snapshot { reason, options } => {
            handle_snapshot(app_context, reason, options)
        }
        DiagnosticCommand::EvaluateInvariants { scope } => {
            let requested_scope = scope.clone();
            let context = app_context();
            match snapshot::evaluate_invariants(&context.shared, scope) {
                Ok(results) => {
                    let failed = results
                        .iter()
                        .filter(|result| result.outcome == InvariantOutcome::Failed)
                        .count();
                    events.record_event(
                        DiagnosticEventFamily::Invariant,
                        "diagnostics.invariants",
                        "evaluated",
                        serde_json::json!({
                            "scope": requested_scope,
                            "total": results.len(),
                            "failed": failed,
                        }),
                    );
                    DiagnosticResponse::InvariantResults { results }
                }
                Err(message) => protocol_error("invariant_evaluation_failed", &message, false),
            }
        }
        DiagnosticCommand::Flush => {
            let summary = events.flush_summary();
            DiagnosticResponse::Flushed {
                events_flushed: summary.visible_events,
                dropped_events: summary.dropped_events,
            }
        }
        DiagnosticCommand::DrainEvents { limit } => {
            let drained = events.drain(limit);
            DiagnosticResponse::Events {
                events: drained.events,
                dropped_events: drained.dropped_events,
            }
        }
        DiagnosticCommand::PrepareDeterministicMode { options } => {
            events.record_log(
                "info",
                "diagnostics.deterministic_mode",
                "prepared",
                serde_json::json!({
                    "disable_animations": options.disable_animations,
                    "disable_background_timers": options.disable_background_timers,
                    "fixed_clock_utc": options.fixed_clock_utc,
                }),
            );
            DiagnosticResponse::Ack {
                message: Some("deterministic mode recorded".to_owned()),
            }
        }
    }
}

fn handle_snapshot(
    app_context: impl FnOnce() -> DiagnosticAppContext,
    reason: String,
    options: SnapshotOptions,
) -> DiagnosticResponse {
    let context = app_context();
    match snapshot::collect_snapshot(
        &context.shared,
        reason,
        context.diagnostic_endpoint,
        &options,
    ) {
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
            "mark_step".to_owned(),
            "clear_step".to_owned(),
            "snapshot".to_owned(),
            "evaluate_invariants".to_owned(),
            "flush".to_owned(),
            "drain_events".to_owned(),
            "prepare_deterministic_mode".to_owned(),
        ],
        event_families: vec![
            DiagnosticEventFamily::TestStep,
            DiagnosticEventFamily::Invariant,
            DiagnosticEventFamily::Log,
        ],
        snapshots: true,
        invariants: true,
        step_markers: true,
        deterministic_mode: true,
        flush: true,
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

    fn event_store() -> DiagnosticEventStore {
        DiagnosticEventStore::with_capacity(16)
    }

    #[test]
    fn hello_reports_protocol_pid_build_features_and_capabilities() {
        let events = event_store();
        let response = handle_request(
            hello_request("secret"),
            "secret",
            &events,
            || unreachable!(),
        );

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
        assert!(capabilities.commands.contains(&"mark_step".to_owned()));
        assert!(capabilities.commands.contains(&"flush".to_owned()));
        assert!(capabilities.commands.contains(&"drain_events".to_owned()));
        assert!(capabilities
            .event_families
            .contains(&DiagnosticEventFamily::TestStep));
        assert!(capabilities
            .event_families
            .contains(&DiagnosticEventFamily::Invariant));
        assert!(capabilities
            .event_families
            .contains(&DiagnosticEventFamily::Log));
        assert!(!capabilities
            .event_families
            .contains(&DiagnosticEventFamily::Render));
        assert!(capabilities.snapshots);
        assert!(capabilities.invariants);
        assert!(capabilities.step_markers);
        assert!(capabilities.flush);
        assert_eq!(capabilities.transports, vec!["named_pipe"]);
    }

    #[test]
    fn missing_or_wrong_token_is_rejected_without_app_state() {
        for token in ["", "wrong"] {
            let events = event_store();
            let response =
                handle_request(hello_request(token), "secret", &events, || unreachable!());

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
    fn wrong_token_cannot_drain_existing_events() {
        let events = event_store();
        events.record_log(
            "warning",
            "diagnostics.test",
            "seeded",
            serde_json::json!({}),
        );

        let response = handle_request(
            DiagnosticRequest {
                token: "wrong".to_owned(),
                command: DiagnosticCommand::DrainEvents { limit: None },
                ..Default::default()
            },
            "secret",
            &events,
            || unreachable!("unauthorized drain must not touch app state"),
        );

        let DiagnosticResponse::Error { error } = response else {
            panic!("expected unauthorized response");
        };
        assert_eq!(error.code, "unauthorized");
        assert!(error.details.as_object().unwrap().is_empty());

        let drained = events.drain(None);
        assert_eq!(drained.events.len(), 1);
        assert_eq!(drained.events[0].payload.kind, "seeded");
    }

    #[test]
    fn snapshot_and_invariant_commands_return_structured_results_after_token_validation() {
        let shared = std::sync::Arc::new(std::sync::Mutex::new(crate::state::seed_state()));
        let events = event_store();

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
            &events,
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
            &events,
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

        let drained = events.drain(None);
        assert_eq!(drained.events.len(), 1);
        assert_eq!(
            drained.events[0].payload.family,
            DiagnosticEventFamily::Invariant
        );
        assert_eq!(drained.events[0].payload.kind, "evaluated");
    }

    #[test]
    fn snapshot_buffer_request_is_opt_in_and_authorized() {
        let shared = std::sync::Arc::new(std::sync::Mutex::new(crate::state::seed_state()));
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
            &event_store(),
            || DiagnosticAppContext {
                shared,
                diagnostic_endpoint: None,
            },
        );

        let DiagnosticResponse::Snapshot { snapshot } = response else {
            panic!("expected snapshot response");
        };
        assert_eq!(snapshot.config["terminal_buffer_contents_included"], true);
    }

    #[test]
    fn mark_step_flush_and_drain_return_correlated_events() {
        let events = event_store();
        let shared = std::sync::Arc::new(std::sync::Mutex::new(crate::state::seed_state()));

        let response = handle_request(
            DiagnosticRequest {
                token: "secret".to_owned(),
                command: DiagnosticCommand::MarkStep {
                    id: "resize-left".to_owned(),
                    label: "Resize left edge".to_owned(),
                },
                ..Default::default()
            },
            "secret",
            &events,
            || unreachable!("mark step must not touch app state"),
        );
        assert!(matches!(response, DiagnosticResponse::Ack { .. }));

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
            &events,
            || unreachable!("deterministic event must not touch app state"),
        );
        let DiagnosticResponse::Ack { message } = response else {
            panic!("expected ack");
        };
        assert_eq!(message.as_deref(), Some("deterministic mode recorded"));

        let response = handle_request(
            DiagnosticRequest {
                token: "secret".to_owned(),
                command: DiagnosticCommand::Flush,
                ..Default::default()
            },
            "secret",
            &events,
            || DiagnosticAppContext {
                shared: shared.clone(),
                diagnostic_endpoint: None,
            },
        );
        let DiagnosticResponse::Flushed {
            events_flushed,
            dropped_events,
        } = response
        else {
            panic!("expected flush response");
        };
        assert_eq!(events_flushed, 2);
        assert_eq!(dropped_events, 0);

        let response = handle_request(
            DiagnosticRequest {
                token: "secret".to_owned(),
                command: DiagnosticCommand::DrainEvents { limit: None },
                ..Default::default()
            },
            "secret",
            &events,
            || DiagnosticAppContext {
                shared,
                diagnostic_endpoint: None,
            },
        );
        let DiagnosticResponse::Events {
            events,
            dropped_events,
        } = response
        else {
            panic!("expected events response");
        };
        assert_eq!(dropped_events, 0);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].seq, 1);
        assert_eq!(events[0].payload.family, DiagnosticEventFamily::TestStep);
        assert_eq!(events[0].test_step_id.as_deref(), Some("resize-left"));
        assert_eq!(events[1].payload.family, DiagnosticEventFamily::Log);
        assert_eq!(events[1].test_step_id.as_deref(), Some("resize-left"));
    }

    #[test]
    fn clear_step_removes_correlation_from_future_events() {
        let events = event_store();

        let _ = handle_request(
            DiagnosticRequest {
                token: "secret".to_owned(),
                command: DiagnosticCommand::MarkStep {
                    id: "first".to_owned(),
                    label: "First".to_owned(),
                },
                ..Default::default()
            },
            "secret",
            &events,
            || unreachable!(),
        );
        let _ = handle_request(
            DiagnosticRequest {
                token: "secret".to_owned(),
                command: DiagnosticCommand::ClearStep { reason: None },
                ..Default::default()
            },
            "secret",
            &events,
            || unreachable!(),
        );
        let _ = handle_request(
            DiagnosticRequest {
                token: "secret".to_owned(),
                command: DiagnosticCommand::PrepareDeterministicMode {
                    options: Default::default(),
                },
                ..Default::default()
            },
            "secret",
            &events,
            || unreachable!(),
        );

        let drained = events.drain(None);
        assert_eq!(drained.events.len(), 3);
        assert_eq!(drained.events[0].test_step_id.as_deref(), Some("first"));
        assert_eq!(drained.events[1].payload.kind, "cleared");
        assert!(drained.events[2].test_step_id.is_none());
    }
}
