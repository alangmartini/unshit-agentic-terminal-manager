use std::io;

use terminal_manager_diagnostics::{
    is_supported_protocol_version, AppIdentity, DiagnosticCapabilities, DiagnosticCommand,
    DiagnosticEventFamily, DiagnosticProtocolError, DiagnosticRequest, DiagnosticResponse,
    DIAGNOSTIC_PROTOCOL_VERSION,
};

use super::config::DiagnosticConfig;
use super::transport;

pub async fn run(config: DiagnosticConfig) -> io::Result<()> {
    transport::run(config).await
}

pub fn handle_request(request: DiagnosticRequest, expected_token: &str) -> DiagnosticResponse {
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
        _ => protocol_error(
            "unsupported_command",
            "only hello is supported in this slice",
            false,
        ),
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
        commands: vec!["hello".to_owned()],
        event_families: Vec::<DiagnosticEventFamily>::new(),
        snapshots: false,
        invariants: false,
        step_markers: false,
        deterministic_mode: false,
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
        let response = handle_request(hello_request("secret"), "secret");

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
        assert_eq!(capabilities.commands, vec!["hello"]);
        assert_eq!(capabilities.transports, vec!["named_pipe"]);
    }

    #[test]
    fn missing_or_wrong_token_is_rejected_without_app_state() {
        for token in ["", "wrong"] {
            let response = handle_request(hello_request(token), "secret");

            let DiagnosticResponse::Error { error } = response else {
                panic!("expected unauthorized response");
            };

            assert_eq!(error.code, "unauthorized");
            assert!(error.details.as_object().unwrap().is_empty());
            assert!(!error.message.contains("terminal-manager"));
            assert!(!error.message.contains(&std::process::id().to_string()));
        }
    }
}
