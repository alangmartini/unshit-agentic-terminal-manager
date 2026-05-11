use terminal_manager_diagnostics::{
    is_supported_protocol_version, DiagnosticCapabilities, DiagnosticCommand, DiagnosticEnvelope,
    DiagnosticEvent, DiagnosticEventFamily, DiagnosticRequest, DiagnosticResponse,
    FailureBundleArtifact, FailureClassification, FailureManifest, InvariantEvaluation,
    InvariantOutcome, ObserveMode, ProtocolCompatibilityError, Rect, ResultStatus, RunInfo,
    RunnerAction, RunnerActionKind, RunnerActionTarget, SnapshotOptions, SuiteFailure, SuiteResult,
    TerminalGridSnapshot, TerminalManagerSnapshot, TestRunResult, DIAGNOSTIC_PROTOCOL_VERSION,
    EVENT_SCHEMA_VERSION, FAILURE_MANIFEST_SCHEMA_VERSION, RESULTS_SCHEMA_VERSION,
    SNAPSHOT_SCHEMA_VERSION,
};

#[test]
fn command_round_trip_uses_snake_case_tags() {
    let envelope = DiagnosticEnvelope {
        schema_version: DIAGNOSTIC_PROTOCOL_VERSION.to_owned(),
        seq: 7,
        timestamp_utc: "2026-05-10T17:30:12Z".to_owned(),
        monotonic_ms: 123,
        test_step_id: None,
        correlation_id: Some("corr-1".to_owned()),
        payload: DiagnosticCommand::MarkStep {
            id: "resize-left".to_owned(),
            label: "Resize from left edge".to_owned(),
        },
    };

    let json = serde_json::to_value(&envelope).unwrap();
    assert_eq!(json["payload"]["type"], "mark_step");
    assert_eq!(json["payload"]["id"], "resize-left");

    let decoded: DiagnosticEnvelope<DiagnosticCommand> = serde_json::from_value(json).unwrap();
    assert_eq!(decoded, envelope);
}

#[test]
fn event_collection_commands_are_versioned_and_additive() {
    let drain = DiagnosticCommand::DrainEvents { limit: Some(25) };
    let drain_json = serde_json::to_value(&drain).unwrap();
    assert_eq!(drain_json["type"], "drain_events");
    assert_eq!(drain_json["limit"], 25);

    let clear = DiagnosticCommand::ClearStep {
        reason: Some("between assertions".to_owned()),
    };
    let clear_json = serde_json::to_value(&clear).unwrap();
    assert_eq!(clear_json["type"], "clear_step");
    assert_eq!(clear_json["reason"], "between assertions");

    let response = DiagnosticResponse::Events {
        events: Vec::new(),
        dropped_events: 3,
    };
    let response_json = serde_json::to_value(response).unwrap();
    assert_eq!(response_json["type"], "events");
    assert_eq!(response_json["dropped_events"], 3);
}

#[test]
fn snapshot_command_defaults_to_excluding_terminal_buffer_contents() {
    let request = serde_json::json!({
        "schema_version": terminal_manager_diagnostics::COMMAND_SCHEMA_VERSION,
        "token": "secret",
        "command": {
            "type": "snapshot",
            "reason": "pre-assertion"
        }
    });

    let decoded: DiagnosticRequest = serde_json::from_value(request).unwrap();
    let DiagnosticCommand::Snapshot { reason, options } = decoded.command else {
        panic!("expected snapshot command");
    };
    assert_eq!(reason, "pre-assertion");
    assert_eq!(options, SnapshotOptions::default());
    assert!(!options.include_terminal_buffer);

    let encoded = serde_json::to_value(DiagnosticCommand::Snapshot {
        reason: "debug".to_owned(),
        options: SnapshotOptions {
            include_terminal_buffer: true,
        },
    })
    .unwrap();
    assert_eq!(encoded["options"]["include_terminal_buffer"], true);
}

#[test]
fn authenticated_request_wraps_command_without_leaking_token_into_command_shape() {
    let request = DiagnosticRequest {
        schema_version: terminal_manager_diagnostics::COMMAND_SCHEMA_VERSION.to_owned(),
        token: "per-run-secret".to_owned(),
        command: DiagnosticCommand::Hello {
            required_protocol_version: Some(DIAGNOSTIC_PROTOCOL_VERSION.to_owned()),
        },
    };

    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(
        json["schema_version"],
        "terminal-manager.diagnostics.command/v1"
    );
    assert_eq!(json["token"], "per-run-secret");
    assert_eq!(json["command"]["type"], "hello");
    assert!(json["command"].get("token").is_none());

    let decoded: DiagnosticRequest = serde_json::from_value(json).unwrap();
    assert_eq!(decoded, request);
}

#[test]
fn event_jsonl_round_trip_preserves_extensible_fields() {
    let event = DiagnosticEnvelope {
        schema_version: EVENT_SCHEMA_VERSION.to_owned(),
        seq: 42,
        timestamp_utc: "2026-05-10T17:30:13Z".to_owned(),
        monotonic_ms: 456,
        test_step_id: Some("snap-left".to_owned()),
        correlation_id: None,
        payload: DiagnosticEvent {
            family: DiagnosticEventFamily::Render,
            thread: "main".to_owned(),
            target: "renderer.surface".to_owned(),
            kind: "surface_resized".to_owned(),
            fields: serde_json::json!({
                "width": 1024,
                "height": 768
            }),
        },
    };

    let line = serde_json::to_string(&event).unwrap();
    assert!(line.contains("\"family\":\"render\""));
    assert!(!line.contains('\n'));

    let decoded: DiagnosticEnvelope<DiagnosticEvent> = serde_json::from_str(&line).unwrap();
    assert_eq!(decoded, event);
}

#[test]
fn snapshot_round_trip_uses_defaults_for_additive_fields() {
    let json = serde_json::json!({
        "schema_version": SNAPSHOT_SCHEMA_VERSION,
        "captured_at_utc": "2026-05-10T17:30:14Z",
        "reason": "failure",
        "app": {
            "pid": 1234,
            "build": "dev"
        },
        "window": {
            "outer_bounds": { "x": 1, "y": 2, "width": 800, "height": 600 },
            "focused": true
        },
        "terminal": {
            "grid": { "rows": 24, "cols": 80 }
        }
    });

    let snapshot: TerminalManagerSnapshot = serde_json::from_value(json).unwrap();
    assert!(snapshot.layout.nodes.is_empty());
    assert!(snapshot.recent_warnings.is_empty());
    assert_eq!(
        snapshot.terminal.grid,
        Some(TerminalGridSnapshot { rows: 24, cols: 80 })
    );

    let encoded = serde_json::to_string(&snapshot).unwrap();
    let decoded: TerminalManagerSnapshot = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, snapshot);
}

#[test]
fn result_round_trip_captures_runner_actions_and_suite_status() {
    let result = TestRunResult {
        schema_version: RESULTS_SCHEMA_VERSION.to_owned(),
        run: RunInfo {
            id: "20260510-143012".to_owned(),
            status: ResultStatus::Failed,
            observe: ObserveMode::Full,
            started_at_utc: "2026-05-10T17:30:12Z".to_owned(),
            finished_at_utc: Some("2026-05-10T17:31:04Z".to_owned()),
            selected_suites: vec!["post-resize-glitches".to_owned()],
        },
        app: None,
        replay: Some(terminal_manager_diagnostics::ResultReplayInfo {
            mode: terminal_manager_diagnostics::ReplayMode::Logical,
            trace: "runner.actions.jsonl".to_owned(),
            validated_actions: 3,
        }),
        summary: Default::default(),
        suites: vec![SuiteResult {
            id: "post-resize-glitches".to_owned(),
            status: ResultStatus::Failed,
            failure: Some(SuiteFailure {
                kind: FailureClassification::CrossLayerInvariant,
                message: "surface height did not match".to_owned(),
                first_bad_signal: Some("render.surface_resized missing".to_owned()),
            }),
            artifacts: vec!["results.json".to_owned()],
            actions: vec![RunnerAction {
                seq: 1,
                timestamp_utc: "2026-05-10T17:30:20Z".to_owned(),
                monotonic_ms: 8000,
                step_id: Some("snap-left".to_owned()),
                target: RunnerActionTarget::Window {
                    title: Some("Terminal Manager".to_owned()),
                    process_id: Some(1234),
                },
                kind: RunnerActionKind::ResizeWindow {
                    bounds: Rect {
                        x: 0,
                        y: 0,
                        width: 1024,
                        height: 768,
                    },
                },
            }],
        }],
    };

    let json = serde_json::to_string_pretty(&result).unwrap();
    assert!(json.contains("\"cross_layer_invariant\""));

    let decoded: TestRunResult = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, result);
}

#[test]
fn failure_manifest_round_trip_captures_evidence_and_invariants() {
    let manifest = FailureManifest {
        schema_version: FAILURE_MANIFEST_SCHEMA_VERSION.to_owned(),
        run_id: "20260510-143012".to_owned(),
        suite_id: "post-resize-glitches".to_owned(),
        classification: FailureClassification::VisualRegression,
        message: "unexpected pixels after snap".to_owned(),
        first_bad_signal: Some("pixel_ratio".to_owned()),
        artifacts: vec![FailureBundleArtifact {
            kind: "screenshot".to_owned(),
            path: "post-resize-glitches-post.png".to_owned(),
        }],
        invariant_results: vec![InvariantEvaluation {
            id: "renderer.surface_matches_window".to_owned(),
            outcome: InvariantOutcome::Failed,
            message: Some("surface height mismatch".to_owned()),
            details: serde_json::json!({ "delta": 12 }),
        }],
        secondary_errors: vec![],
    };

    let encoded = serde_json::to_string(&manifest).unwrap();
    let decoded: FailureManifest = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, manifest);
}

#[test]
fn compatibility_helpers_accept_known_versions_and_reject_unknown_required_versions() {
    assert!(is_supported_protocol_version(DIAGNOSTIC_PROTOCOL_VERSION).is_ok());

    let error = is_supported_protocol_version("terminal-manager.diagnostics/v99").unwrap_err();
    assert_eq!(
        error,
        ProtocolCompatibilityError::UnsupportedRequiredVersion {
            requested: "terminal-manager.diagnostics/v99".to_owned(),
            supported: vec![DIAGNOSTIC_PROTOCOL_VERSION.to_owned()],
        }
    );

    let hello = DiagnosticResponse::Hello {
        protocol_version: DIAGNOSTIC_PROTOCOL_VERSION.to_owned(),
        enabled_features: vec!["diagnostics".to_owned()],
        app: Default::default(),
        capabilities: DiagnosticCapabilities::default(),
    };
    let json = serde_json::to_value(hello).unwrap();
    assert_eq!(json["type"], "hello");
    assert_eq!(json["enabled_features"][0], "diagnostics");
}
