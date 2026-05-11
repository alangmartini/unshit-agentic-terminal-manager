//! Shared diagnostic schema for the desktop regression observability harness.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fmt;

pub const DIAGNOSTIC_PROTOCOL_VERSION: &str = "terminal-manager.diagnostics/v1";
pub const COMMAND_SCHEMA_VERSION: &str = "terminal-manager.diagnostics.command/v1";
pub const RESPONSE_SCHEMA_VERSION: &str = "terminal-manager.diagnostics.response/v1";
pub const EVENT_SCHEMA_VERSION: &str = "terminal-manager.diagnostics.event/v1";
pub const SNAPSHOT_SCHEMA_VERSION: &str = "terminal-manager.diagnostics.snapshot/v1";
pub const RESULTS_SCHEMA_VERSION: &str = "desktop-regression.results/v2";
pub const FAILURE_MANIFEST_SCHEMA_VERSION: &str = "desktop-regression.failure-manifest/v1";
pub const RUNNER_ACTION_SCHEMA_VERSION: &str = "desktop-regression.runner-action/v1";

pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &[DIAGNOSTIC_PROTOCOL_VERSION];
pub const SUPPORTED_SNAPSHOT_SCHEMA_VERSIONS: &[&str] = &[SNAPSHOT_SCHEMA_VERSION];
pub const SUPPORTED_RESULTS_SCHEMA_VERSIONS: &[&str] = &[RESULTS_SCHEMA_VERSION];
pub const SUPPORTED_FAILURE_MANIFEST_SCHEMA_VERSIONS: &[&str] = &[FAILURE_MANIFEST_SCHEMA_VERSION];

pub type JsonObject = Map<String, Value>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolCompatibilityError {
    UnsupportedRequiredVersion {
        requested: String,
        supported: Vec<String>,
    },
}

impl fmt::Display for ProtocolCompatibilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtocolCompatibilityError::UnsupportedRequiredVersion {
                requested,
                supported,
            } => {
                write!(
                    f,
                    "unsupported required protocol/schema version {requested}; supported: {}",
                    supported.join(", ")
                )
            }
        }
    }
}

impl std::error::Error for ProtocolCompatibilityError {}

pub fn is_supported_protocol_version(version: &str) -> Result<(), ProtocolCompatibilityError> {
    require_supported_version(version, SUPPORTED_PROTOCOL_VERSIONS)
}

pub fn is_supported_snapshot_schema_version(
    version: &str,
) -> Result<(), ProtocolCompatibilityError> {
    require_supported_version(version, SUPPORTED_SNAPSHOT_SCHEMA_VERSIONS)
}

pub fn is_supported_results_schema_version(
    version: &str,
) -> Result<(), ProtocolCompatibilityError> {
    require_supported_version(version, SUPPORTED_RESULTS_SCHEMA_VERSIONS)
}

pub fn is_supported_failure_manifest_schema_version(
    version: &str,
) -> Result<(), ProtocolCompatibilityError> {
    require_supported_version(version, SUPPORTED_FAILURE_MANIFEST_SCHEMA_VERSIONS)
}

fn require_supported_version(
    requested: &str,
    supported: &[&str],
) -> Result<(), ProtocolCompatibilityError> {
    if supported.contains(&requested) {
        Ok(())
    } else {
        Err(ProtocolCompatibilityError::UnsupportedRequiredVersion {
            requested: requested.to_owned(),
            supported: supported
                .iter()
                .map(|version| (*version).to_owned())
                .collect(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticEnvelope<T> {
    pub schema_version: String,
    pub seq: u64,
    pub timestamp_utc: String,
    pub monotonic_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_step_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    pub payload: T,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticRequest {
    #[serde(default = "default_command_schema_version")]
    pub schema_version: String,
    pub token: String,
    pub command: DiagnosticCommand,
}

impl Default for DiagnosticRequest {
    fn default() -> Self {
        Self {
            schema_version: COMMAND_SCHEMA_VERSION.to_owned(),
            token: String::new(),
            command: DiagnosticCommand::Hello {
                required_protocol_version: None,
            },
        }
    }
}

fn default_command_schema_version() -> String {
    COMMAND_SCHEMA_VERSION.to_owned()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DiagnosticCommand {
    Hello {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        required_protocol_version: Option<String>,
    },
    MarkStep {
        id: String,
        label: String,
    },
    ClearStep {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Snapshot {
        reason: String,
        #[serde(default)]
        options: SnapshotOptions,
    },
    EvaluateInvariants {
        scope: InvariantScope,
    },
    Flush,
    DrainEvents {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    },
    PrepareDeterministicMode {
        #[serde(default)]
        options: DeterministicModeOptions,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvariantScope {
    All,
    Window,
    Layout,
    Renderer,
    Terminal,
    Pty,
    Input,
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DeterministicModeOptions {
    pub disable_animations: bool,
    pub disable_background_timers: bool,
    pub fixed_clock_utc: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SnapshotOptions {
    pub include_terminal_buffer: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DiagnosticResponse {
    Hello {
        protocol_version: String,
        #[serde(default)]
        enabled_features: Vec<String>,
        #[serde(default)]
        app: AppIdentity,
        #[serde(default)]
        capabilities: DiagnosticCapabilities,
    },
    Ack {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    Snapshot {
        snapshot: TerminalManagerSnapshot,
    },
    InvariantResults {
        results: Vec<InvariantEvaluation>,
    },
    Flushed {
        events_flushed: u64,
        dropped_events: u64,
    },
    Events {
        events: Vec<DiagnosticEnvelope<DiagnosticEvent>>,
        dropped_events: u64,
    },
    Error {
        error: DiagnosticProtocolError,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiagnosticProtocolError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub details: Value,
}

impl Default for DiagnosticProtocolError {
    fn default() -> Self {
        Self {
            code: "unknown".to_owned(),
            message: String::new(),
            retryable: false,
            details: Value::Object(JsonObject::new()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppIdentity {
    pub name: String,
    pub version: String,
    pub build: String,
    pub commit: Option<String>,
    pub process_id: Option<u32>,
}

impl Default for AppIdentity {
    fn default() -> Self {
        Self {
            name: "terminal-manager".to_owned(),
            version: String::new(),
            build: String::new(),
            commit: None,
            process_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiagnosticCapabilities {
    pub supported_protocol_versions: Vec<String>,
    pub transports: Vec<String>,
    pub commands: Vec<String>,
    pub event_families: Vec<DiagnosticEventFamily>,
    pub snapshots: bool,
    pub invariants: bool,
    pub step_markers: bool,
    pub deterministic_mode: bool,
    pub flush: bool,
}

impl Default for DiagnosticCapabilities {
    fn default() -> Self {
        Self {
            supported_protocol_versions: SUPPORTED_PROTOCOL_VERSIONS
                .iter()
                .map(|version| (*version).to_owned())
                .collect(),
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
            event_families: DiagnosticEventFamily::all().to_vec(),
            snapshots: true,
            invariants: true,
            step_markers: true,
            deterministic_mode: true,
            flush: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticEventFamily {
    TestStep,
    Window,
    Layout,
    Render,
    Terminal,
    Pty,
    Input,
    State,
    Invariant,
    Log,
}

impl DiagnosticEventFamily {
    pub const fn all() -> &'static [Self] {
        &[
            Self::TestStep,
            Self::Window,
            Self::Layout,
            Self::Render,
            Self::Terminal,
            Self::Pty,
            Self::Input,
            Self::State,
            Self::Invariant,
            Self::Log,
        ]
    }
}

impl Default for DiagnosticEventFamily {
    fn default() -> Self {
        Self::State
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiagnosticEvent {
    pub family: DiagnosticEventFamily,
    pub thread: String,
    pub target: String,
    pub kind: String,
    pub fields: Value,
}

impl Default for DiagnosticEvent {
    fn default() -> Self {
        Self {
            family: DiagnosticEventFamily::State,
            thread: String::new(),
            target: String::new(),
            kind: String::new(),
            fields: Value::Object(JsonObject::new()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalManagerSnapshot {
    pub schema_version: String,
    pub captured_at_utc: String,
    pub reason: String,
    pub app: SnapshotAppIdentity,
    pub window: WindowSnapshot,
    pub layout: LayoutSnapshot,
    pub terminal: TerminalSnapshot,
    pub renderer: RendererSnapshot,
    pub pty: PtySnapshot,
    pub input: InputSnapshot,
    pub config: Value,
    pub recent_warnings: Vec<DiagnosticLogRecord>,
    pub recent_errors: Vec<DiagnosticLogRecord>,
}

impl Default for TerminalManagerSnapshot {
    fn default() -> Self {
        Self {
            schema_version: SNAPSHOT_SCHEMA_VERSION.to_owned(),
            captured_at_utc: String::new(),
            reason: String::new(),
            app: SnapshotAppIdentity::default(),
            window: WindowSnapshot::default(),
            layout: LayoutSnapshot::default(),
            terminal: TerminalSnapshot::default(),
            renderer: RendererSnapshot::default(),
            pty: PtySnapshot::default(),
            input: InputSnapshot::default(),
            config: Value::Object(JsonObject::new()),
            recent_warnings: Vec::new(),
            recent_errors: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SnapshotAppIdentity {
    pub pid: Option<u32>,
    pub build: String,
    pub commit: Option<String>,
    pub diagnostic_endpoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowSnapshot {
    pub outer_bounds: Option<Rect>,
    pub client_bounds: Option<Rect>,
    pub scale_factor: Option<f64>,
    pub focused: bool,
    pub resize_generation: Option<u64>,
}

impl Default for WindowSnapshot {
    fn default() -> Self {
        Self {
            outer_bounds: None,
            client_bounds: None,
            scale_factor: None,
            focused: false,
            resize_generation: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LayoutSnapshot {
    pub nodes: Vec<LayoutNodeSnapshot>,
    pub dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutNodeSnapshot {
    pub id: String,
    pub label: Option<String>,
    pub bounds: Option<Rect>,
    pub visible: bool,
    pub z_order: i32,
    pub dirty: bool,
}

impl Default for LayoutNodeSnapshot {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: None,
            bounds: None,
            visible: true,
            z_order: 0,
            dirty: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TerminalSnapshot {
    pub grid: Option<TerminalGridSnapshot>,
    pub visible_rows: Option<u32>,
    pub scrollback_len: Option<u64>,
    pub cursor: Option<TerminalCursorSnapshot>,
    pub selection_active: bool,
    pub active_session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalGridSnapshot {
    pub rows: u32,
    pub cols: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalCursorSnapshot {
    pub row: u32,
    pub col: u32,
    #[serde(default)]
    pub visible: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RendererSnapshot {
    pub surface_size: Option<Size>,
    pub frame_counter: Option<u64>,
    pub last_present_time_utc: Option<String>,
    pub dirty_regions: Vec<Rect>,
    pub cached_layers: Vec<String>,
    pub glyph_atlas: Option<GlyphAtlasSnapshot>,
    pub last_render_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlyphAtlasSnapshot {
    pub pages: u32,
    pub glyphs: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PtySnapshot {
    pub sessions: Vec<PtySessionSnapshot>,
    pub pending_writes: u64,
    pub recent_events: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PtySessionSnapshot {
    pub id: String,
    pub name: Option<String>,
    pub process_id: Option<u32>,
    pub status: String,
    pub reconnecting: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct InputSnapshot {
    pub focused_element: Option<String>,
    pub pressed_modifiers: Vec<String>,
    pub pointer_capture: Option<String>,
    pub hover_target: Option<String>,
    pub drag_state: Option<String>,
    pub resize_handle: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiagnosticLogRecord {
    pub timestamp_utc: String,
    pub level: String,
    pub target: String,
    pub message: String,
    pub fields: Value,
}

impl Default for DiagnosticLogRecord {
    fn default() -> Self {
        Self {
            timestamp_utc: String::new(),
            level: String::new(),
            target: String::new(),
            message: String::new(),
            fields: Value::Object(JsonObject::new()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvariantOutcome {
    Passed,
    Failed,
    Skipped,
}

impl Default for InvariantOutcome {
    fn default() -> Self {
        Self::Skipped
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InvariantEvaluation {
    pub id: String,
    pub outcome: InvariantOutcome,
    pub message: Option<String>,
    pub details: Value,
}

impl Default for InvariantEvaluation {
    fn default() -> Self {
        Self {
            id: String::new(),
            outcome: InvariantOutcome::Skipped,
            message: None,
            details: Value::Object(JsonObject::new()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunnerAction {
    pub seq: u64,
    pub timestamp_utc: String,
    pub monotonic_ms: u64,
    pub step_id: Option<String>,
    pub target: RunnerActionTarget,
    pub kind: RunnerActionKind,
}

impl Default for RunnerAction {
    fn default() -> Self {
        Self {
            seq: 0,
            timestamp_utc: String::new(),
            monotonic_ms: 0,
            step_id: None,
            target: RunnerActionTarget::None,
            kind: RunnerActionKind::Note {
                message: String::new(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunnerActionTarget {
    None,
    Desktop,
    Window {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        process_id: Option<u32>,
    },
}

impl Default for RunnerActionTarget {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunnerActionKind {
    MarkStep {
        id: String,
    },
    MoveWindow {
        bounds: Rect,
    },
    ResizeWindow {
        bounds: Rect,
    },
    SendKeys {
        keys: Vec<String>,
    },
    Mouse {
        x: i32,
        y: i32,
        button: Option<String>,
    },
    Wait {
        reason: String,
        timeout_ms: u64,
    },
    Screenshot {
        path: String,
    },
    Note {
        message: String,
    },
}

impl Default for RunnerActionKind {
    fn default() -> Self {
        Self::Note {
            message: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TestRunResult {
    pub schema_version: String,
    pub run: RunInfo,
    pub app: Option<ResultAppInfo>,
    pub summary: ResultSummary,
    pub suites: Vec<SuiteResult>,
}

impl Default for TestRunResult {
    fn default() -> Self {
        Self {
            schema_version: RESULTS_SCHEMA_VERSION.to_owned(),
            run: RunInfo::default(),
            app: None,
            summary: ResultSummary::default(),
            suites: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunInfo {
    pub id: String,
    pub status: ResultStatus,
    pub observe: ObserveMode,
    pub started_at_utc: String,
    pub finished_at_utc: Option<String>,
    pub selected_suites: Vec<String>,
}

impl Default for RunInfo {
    fn default() -> Self {
        Self {
            id: String::new(),
            status: ResultStatus::Skipped,
            observe: ObserveMode::Off,
            started_at_utc: String::new(),
            finished_at_utc: None,
            selected_suites: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ResultAppInfo {
    pub binary: String,
    pub sha256: Option<String>,
    pub pid: Option<u32>,
    pub diagnostics: Option<ResultDiagnosticInfo>,
}

impl Default for ResultAppInfo {
    fn default() -> Self {
        Self {
            binary: String::new(),
            sha256: None,
            pid: None,
            diagnostics: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ResultDiagnosticInfo {
    pub enabled: bool,
    pub protocol_version: Option<String>,
    pub transport: Option<String>,
}

impl Default for ResultDiagnosticInfo {
    fn default() -> Self {
        Self {
            enabled: false,
            protocol_version: None,
            transport: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ResultSummary {
    pub total: u32,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObserveMode {
    Off,
    Basic,
    Full,
}

impl Default for ObserveMode {
    fn default() -> Self {
        Self::Off
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultStatus {
    Passed,
    Failed,
    Skipped,
}

impl Default for ResultStatus {
    fn default() -> Self {
        Self::Skipped
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SuiteResult {
    pub id: String,
    pub status: ResultStatus,
    pub failure: Option<SuiteFailure>,
    pub artifacts: Vec<String>,
    pub actions: Vec<RunnerAction>,
}

impl Default for SuiteResult {
    fn default() -> Self {
        Self {
            id: String::new(),
            status: ResultStatus::Skipped,
            failure: None,
            artifacts: Vec::new(),
            actions: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SuiteFailure {
    pub kind: FailureClassification,
    pub message: String,
    pub first_bad_signal: Option<String>,
}

impl Default for SuiteFailure {
    fn default() -> Self {
        Self {
            kind: FailureClassification::Unknown,
            message: String::new(),
            first_bad_signal: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClassification {
    Unknown,
    Setup,
    AppCrash,
    Protocol,
    Assertion,
    VisualRegression,
    CrossLayerInvariant,
    Timeout,
    Artifact,
}

impl Default for FailureClassification {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FailureManifest {
    pub schema_version: String,
    pub run_id: String,
    pub suite_id: String,
    pub classification: FailureClassification,
    pub message: String,
    pub first_bad_signal: Option<String>,
    pub artifacts: Vec<FailureBundleArtifact>,
    pub invariant_results: Vec<InvariantEvaluation>,
    pub secondary_errors: Vec<String>,
}

impl Default for FailureManifest {
    fn default() -> Self {
        Self {
            schema_version: FAILURE_MANIFEST_SCHEMA_VERSION.to_owned(),
            run_id: String::new(),
            suite_id: String::new(),
            classification: FailureClassification::Unknown,
            message: String::new(),
            first_bad_signal: None,
            artifacts: Vec::new(),
            invariant_results: Vec::new(),
            secondary_errors: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FailureBundleArtifact {
    pub kind: String,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Default for Rect {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Default for Size {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
        }
    }
}
