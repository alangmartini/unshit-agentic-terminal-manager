//! Flow Explorer data model.
//!
//! A *flow* is an agent-authored JSON document describing one user-facing
//! path through a codebase: nodes (functions, events, state) tagged with the
//! process they run in and the carrier an event travels over, plus an
//! ordered edge list. Structure has one source of truth, `entries` +
//! `edges` (array order = call order); the call-stack tree, the Miller
//! columns and the graph are all derived from it.
//!
//! Parsing is two-phase ([`parse_flow`]): a tiny envelope first, so a
//! producer that only managed to write `{schema_version, error}` surfaces
//! its own reason instead of a "missing field" error, then the full
//! document, path normalization and validation.

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Bump when the on-disk shape changes incompatibly.
pub const FLOW_SCHEMA_VERSION: u32 = 1;

/// What the producer was asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowMode {
    /// Explain how one flow works on the current tree.
    #[default]
    Explain,
    /// Explain the flows a change touches, with per-node diff status.
    Review,
}

impl FlowMode {
    pub fn as_str(self) -> &'static str {
        match self {
            FlowMode::Explain => "explain",
            FlowMode::Review => "review",
        }
    }
}

/// Git range a review-mode flow was authored against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffRange {
    pub base: String,
    pub head: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Function,
    Event,
    State,
}

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NodeKind::Function => "function",
            NodeKind::Event => "event",
            NodeKind::State => "state",
        }
    }
}

/// How an event travels between two functions (the legend's "event carrier").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Carrier {
    Ui,
    Ipc,
    Rpc,
    Http,
    Fs,
    Process,
    Network,
    InMemory,
}

impl Carrier {
    /// Legend order.
    pub const ALL: [Carrier; 8] = [
        Carrier::Ui,
        Carrier::Ipc,
        Carrier::Rpc,
        Carrier::Http,
        Carrier::Fs,
        Carrier::Process,
        Carrier::Network,
        Carrier::InMemory,
    ];

    /// Legend label, as shown in the reference UI.
    pub fn label(self) -> &'static str {
        match self {
            Carrier::Ui => "UI",
            Carrier::Ipc => "IPC",
            Carrier::Rpc => "RPC",
            Carrier::Http => "HTTP",
            Carrier::Fs => "FS",
            Carrier::Process => "process",
            Carrier::Network => "network",
            Carrier::InMemory => "in-memory",
        }
    }

    /// Stable identifier for CSS class names and telemetry.
    pub fn slug(self) -> &'static str {
        match self {
            Carrier::Ui => "ui",
            Carrier::Ipc => "ipc",
            Carrier::Rpc => "rpc",
            Carrier::Http => "http",
            Carrier::Fs => "fs",
            Carrier::Process => "process",
            Carrier::Network => "network",
            Carrier::InMemory => "in-memory",
        }
    }
}

/// Per-node change status in review mode. `Same` for explain-mode flows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffStatus {
    #[default]
    Same,
    Added,
    Removed,
    Modified,
}

impl DiffStatus {
    /// Rail marker (`+`, `-`, `~`); `None` for unchanged nodes.
    pub fn marker(self) -> Option<&'static str> {
        match self {
            DiffStatus::Same => None,
            DiffStatus::Added => Some("+"),
            DiffStatus::Removed => Some("-"),
            DiffStatus::Modified => Some("~"),
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            DiffStatus::Same => "same",
            DiffStatus::Added => "added",
            DiffStatus::Removed => "removed",
            DiffStatus::Modified => "modified",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// function → function, or function → event it emits.
    Calls,
    /// event → the function that handles it.
    HandledBy,
    /// function → the reply event that resolves an earlier request.
    Resolves,
}

impl EdgeKind {
    /// Section heading / connector label, lowercase like the rest of the UI.
    pub fn label(self) -> &'static str {
        match self {
            EdgeKind::Calls => "calls",
            EdgeKind::HandledBy => "handled by",
            EdgeKind::Resolves => "resolves",
        }
    }
}

/// A swim lane: where code runs (`outside`, `renderer`, `main`, ...).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Process {
    pub id: String,
    pub label: String,
}

/// Source location relative to [`Flow::repo_root`], 1-based lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub file: String,
    pub line: u32,
    #[serde(default)]
    pub end_line: Option<u32>,
}

impl Location {
    /// `file:line` or `file:line-end` as shown next to a row.
    pub fn display(&self) -> String {
        match self.end_line {
            Some(end) if end > self.line => format!("{}:{}-{}", self.file, self.line, end),
            _ => format!("{}:{}", self.file, self.line),
        }
    }

    /// Producers on Windows commonly emit `src\ui\foo.rs`; the model stores
    /// forward slashes only so display, validation and lookup agree.
    pub fn normalize(&mut self) {
        if self.file.contains('\\') {
            self.file = self.file.replace('\\', "/");
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub name: String,
    pub kind: NodeKind,
    /// `Process::id`; events without one are pure edges in the graph.
    #[serde(default)]
    pub process: Option<String>,
    /// Events only.
    #[serde(default)]
    pub carrier: Option<Carrier>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub location: Option<Location>,
    #[serde(default)]
    pub status: DiffStatus,
    /// Children the producer pruned; rendered as `[+n]`.
    #[serde(default)]
    pub hidden_children: u32,
    /// Events: what crosses the carrier, e.g. `{ sessionId, text }`.
    #[serde(default)]
    pub payload: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flow {
    pub schema_version: u32,
    pub title: String,
    #[serde(default)]
    pub summary: String,
    /// Directory every [`Location::file`] is relative to. Absolute, or
    /// relative to the directory holding the flow JSON (fixtures); resolve
    /// with [`resolve_repo_root`].
    pub repo_root: String,
    /// e.g. `feat/x@a1b2c3d`; informational.
    #[serde(default)]
    pub git_ref: Option<String>,
    #[serde(default)]
    pub mode: FlowMode,
    #[serde(default)]
    pub diff_range: Option<DiffRange>,
    /// Producer failure. Checked by [`parse_flow`] before the body is read.
    #[serde(default)]
    pub error: Option<String>,
    pub processes: Vec<Process>,
    pub nodes: Vec<Node>,
    /// Ordered: array order is call order.
    pub edges: Vec<Edge>,
    /// Root node ids.
    pub entries: Vec<String>,
    /// Prose pointer to the flow that continues this one.
    #[serde(default)]
    pub next_flow: Option<String>,
}

/// The minimal shape every producer output has, even on failure.
#[derive(Debug, Clone, Deserialize)]
pub struct FlowEnvelope {
    pub schema_version: u32,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowValidationError {
    UnsupportedSchemaVersion {
        found: u32,
    },
    /// Empty, or contains a character the dispatch grammar reserves.
    InvalidId {
        kind: &'static str,
        id: String,
    },
    DuplicateNodeId(String),
    DuplicateProcessId(String),
    UnknownProcessRef {
        node: String,
        process: String,
    },
    UnknownNodeRef {
        context: &'static str,
        id: String,
    },
    EmptyEntries,
    /// Absolute, drive/UNC-prefixed, or escapes the repo with `..`.
    UnsafeLocationPath {
        node: String,
        file: String,
    },
}

impl fmt::Display for FlowValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FlowValidationError::UnsupportedSchemaVersion { found } => write!(
                f,
                "unsupported schema_version {} (expected {})",
                found, FLOW_SCHEMA_VERSION
            ),
            FlowValidationError::InvalidId { kind, id } => {
                write!(f, "invalid {} id {:?}", kind, id)
            }
            FlowValidationError::DuplicateNodeId(id) => write!(f, "duplicate node id {:?}", id),
            FlowValidationError::DuplicateProcessId(id) => {
                write!(f, "duplicate process id {:?}", id)
            }
            FlowValidationError::UnknownProcessRef { node, process } => {
                write!(
                    f,
                    "node {:?} references unknown process {:?}",
                    node, process
                )
            }
            FlowValidationError::UnknownNodeRef { context, id } => {
                write!(f, "{} references unknown node {:?}", context, id)
            }
            FlowValidationError::EmptyEntries => write!(f, "entries is empty"),
            FlowValidationError::UnsafeLocationPath { node, file } => {
                write!(f, "node {:?} has an unsafe location path {:?}", node, file)
            }
        }
    }
}

/// Why a flow file could not be turned into a [`Flow`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowParseError {
    InvalidJson(String),
    UnsupportedSchemaVersion(u32),
    /// The producer wrote `error` instead of a flow.
    ProducerError(String),
    Validation(Vec<FlowValidationError>),
}

impl FlowParseError {
    /// Machine-readable reason for telemetry.
    pub fn reason(&self) -> &'static str {
        match self {
            FlowParseError::InvalidJson(_) => "invalid_json",
            FlowParseError::UnsupportedSchemaVersion(_) => "unsupported_schema_version",
            FlowParseError::ProducerError(_) => "producer_error",
            FlowParseError::Validation(_) => "validation",
        }
    }
}

impl fmt::Display for FlowParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FlowParseError::InvalidJson(detail) => {
                write!(f, "flow is not valid JSON: {}", detail)
            }
            FlowParseError::UnsupportedSchemaVersion(found) => write!(
                f,
                "flow schema_version {} is not supported (expected {})",
                found, FLOW_SCHEMA_VERSION
            ),
            FlowParseError::ProducerError(reason) => {
                write!(f, "agent could not build the flow: {}", reason)
            }
            FlowParseError::Validation(errors) => {
                let shown: Vec<String> = errors.iter().take(3).map(|e| e.to_string()).collect();
                write!(f, "flow failed validation: {}", shown.join("; "))?;
                if errors.len() > 3 {
                    write!(f, " (+{} more)", errors.len() - 3)?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for FlowParseError {}

/// Ids appear inside dispatch strings (`flow.select:<col>:<id>`), which the
/// startup driver splits on `;`; keep those characters out of ids.
fn valid_id(id: &str) -> bool {
    !id.trim().is_empty() && !id.contains([';', '\n', '\r'])
}

/// A location path must stay inside the repo: relative, no `..`, no drive
/// or UNC prefix. Expects forward slashes (see [`Location::normalize`]).
pub fn is_safe_relative_path(file: &str) -> bool {
    if file.is_empty() || file.contains('\0') || file.starts_with('/') {
        return false;
    }
    if file.as_bytes().get(1) == Some(&b':') {
        return false;
    }
    !file.split('/').any(|component| component == "..")
}

impl Flow {
    /// Canonicalize producer output before validation (backslash paths).
    pub fn normalize(&mut self) {
        for node in &mut self.nodes {
            if let Some(location) = &mut node.location {
                location.normalize();
            }
        }
    }

    /// Collects every problem rather than stopping at the first, so one
    /// toast can name them all. Cycles are allowed; tree derivation marks
    /// repeats.
    pub fn validate(&self) -> Result<(), Vec<FlowValidationError>> {
        let mut errors = Vec::new();
        if self.schema_version != FLOW_SCHEMA_VERSION {
            errors.push(FlowValidationError::UnsupportedSchemaVersion {
                found: self.schema_version,
            });
        }

        let mut process_ids: HashSet<&str> = HashSet::new();
        for process in &self.processes {
            if !valid_id(&process.id) {
                errors.push(FlowValidationError::InvalidId {
                    kind: "process",
                    id: process.id.clone(),
                });
                continue;
            }
            if !process_ids.insert(process.id.as_str()) {
                errors.push(FlowValidationError::DuplicateProcessId(process.id.clone()));
            }
        }

        let mut node_ids: HashSet<&str> = HashSet::new();
        for node in &self.nodes {
            if !valid_id(&node.id) {
                errors.push(FlowValidationError::InvalidId {
                    kind: "node",
                    id: node.id.clone(),
                });
                continue;
            }
            if !node_ids.insert(node.id.as_str()) {
                errors.push(FlowValidationError::DuplicateNodeId(node.id.clone()));
            }
            if let Some(process) = &node.process {
                if !process_ids.contains(process.as_str()) {
                    errors.push(FlowValidationError::UnknownProcessRef {
                        node: node.id.clone(),
                        process: process.clone(),
                    });
                }
            }
            if let Some(location) = &node.location {
                if !is_safe_relative_path(&location.file) {
                    errors.push(FlowValidationError::UnsafeLocationPath {
                        node: node.id.clone(),
                        file: location.file.clone(),
                    });
                }
            }
        }

        for edge in &self.edges {
            for id in [&edge.from, &edge.to] {
                if !node_ids.contains(id.as_str()) {
                    errors.push(FlowValidationError::UnknownNodeRef {
                        context: "edge",
                        id: id.clone(),
                    });
                }
            }
        }

        if self.entries.is_empty() {
            errors.push(FlowValidationError::EmptyEntries);
        }
        for id in &self.entries {
            if !node_ids.contains(id.as_str()) {
                errors.push(FlowValidationError::UnknownNodeRef {
                    context: "entries",
                    id: id.clone(),
                });
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn process(&self, id: &str) -> Option<&Process> {
        self.processes.iter().find(|p| p.id == id)
    }

    /// Lane index of a process id in declaration order.
    pub fn process_index(&self, id: &str) -> Option<usize> {
        self.processes.iter().position(|p| p.id == id)
    }

    /// Edges leaving `id`, in array order, with their index into `edges`.
    pub fn outgoing(&self, id: &str) -> Vec<(usize, &Edge)> {
        self.edges
            .iter()
            .enumerate()
            .filter(|(_, e)| e.from == id)
            .collect()
    }

    /// Edges arriving at `id`, in array order, with their index into `edges`.
    pub fn incoming(&self, id: &str) -> Vec<(usize, &Edge)> {
        self.edges
            .iter()
            .enumerate()
            .filter(|(_, e)| e.to == id)
            .collect()
    }
}

/// Envelope first, then the body. See the module docs for why.
pub fn parse_flow(bytes: &[u8]) -> Result<Flow, FlowParseError> {
    let envelope: FlowEnvelope =
        serde_json::from_slice(bytes).map_err(|e| FlowParseError::InvalidJson(e.to_string()))?;
    if envelope.schema_version != FLOW_SCHEMA_VERSION {
        return Err(FlowParseError::UnsupportedSchemaVersion(
            envelope.schema_version,
        ));
    }
    if let Some(error) = envelope.error.filter(|e| !e.trim().is_empty()) {
        return Err(FlowParseError::ProducerError(error));
    }
    let mut flow: Flow =
        serde_json::from_slice(bytes).map_err(|e| FlowParseError::InvalidJson(e.to_string()))?;
    flow.normalize();
    flow.validate().map_err(FlowParseError::Validation)?;
    Ok(flow)
}

/// Absolute `repo_root` is used as-is; a relative one is taken from the
/// directory that holds the flow file (so committed fixtures work on any
/// checkout). `has_root` covers `/x` on Windows, which `is_absolute` does not.
pub fn resolve_repo_root(flow_path: &Path, repo_root: &str) -> PathBuf {
    let root = Path::new(repo_root);
    if root.is_absolute() || root.has_root() {
        return root.to_path_buf();
    }
    flow_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_explorer::test_support::{fixture_path, load_fixture};

    fn minimal(nodes: &[&str], edges: &[(&str, &str)], entries: &[&str]) -> Flow {
        Flow {
            schema_version: FLOW_SCHEMA_VERSION,
            title: "t".into(),
            summary: String::new(),
            repo_root: ".".into(),
            git_ref: None,
            mode: FlowMode::Explain,
            diff_range: None,
            error: None,
            processes: vec![Process {
                id: "main".into(),
                label: "Main".into(),
            }],
            nodes: nodes
                .iter()
                .map(|id| Node {
                    id: (*id).into(),
                    name: (*id).into(),
                    kind: NodeKind::Function,
                    process: Some("main".into()),
                    carrier: None,
                    description: String::new(),
                    tags: vec![],
                    location: None,
                    status: DiffStatus::Same,
                    hidden_children: 0,
                    payload: None,
                })
                .collect(),
            edges: edges
                .iter()
                .map(|(from, to)| Edge {
                    from: (*from).into(),
                    to: (*to).into(),
                    kind: EdgeKind::Calls,
                    label: None,
                })
                .collect(),
            entries: entries.iter().map(|s| (*s).to_string()).collect(),
            next_flow: None,
        }
    }

    #[test]
    fn fixture_parses_and_validates() {
        let flow = load_fixture();
        assert_eq!(flow.title, "Send a prompt");
        assert_eq!(flow.nodes.len(), 11);
        assert_eq!(flow.edges.len(), 10);
        assert_eq!(flow.entries, vec!["ui.cmd-enter".to_string()]);
        assert_eq!(flow.processes.len(), 4);
        assert_eq!(flow.mode, FlowMode::Explain);
        let entry = flow.node("ui.cmd-enter").unwrap();
        assert_eq!(entry.kind, NodeKind::Event);
        assert_eq!(entry.carrier, Some(Carrier::Ui));
        let rpc = flow.node("rpc.sessions.prompt").unwrap();
        assert_eq!(rpc.hidden_children, 1);
        assert!(rpc.process.is_none());
    }

    #[test]
    fn fixture_repo_root_resolves_next_to_the_json() {
        let flow = load_fixture();
        let root = resolve_repo_root(&fixture_path(), &flow.repo_root);
        assert!(root.is_dir(), "{}", root.display());
        for node in &flow.nodes {
            if let Some(location) = &node.location {
                assert!(root.join(&location.file).is_file(), "{}", location.file);
            }
        }
    }

    #[test]
    fn producer_error_document_reports_producer_error() {
        let bytes = br#"{"schema_version":1,"title":"x","error":"no entry points matched"}"#;
        match parse_flow(bytes) {
            Err(FlowParseError::ProducerError(reason)) => {
                assert_eq!(reason, "no entry points matched")
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn blank_error_field_is_ignored() {
        let mut flow = minimal(&["a"], &[], &["a"]);
        flow.error = Some("   ".into());
        let bytes = serde_json::to_vec(&flow).unwrap();
        assert!(parse_flow(&bytes).is_ok());
    }

    #[test]
    fn unsupported_version_wins_over_missing_fields() {
        let bytes = br#"{"schema_version":99}"#;
        assert_eq!(
            parse_flow(bytes),
            Err(FlowParseError::UnsupportedSchemaVersion(99))
        );
    }

    #[test]
    fn invalid_json_and_reasons() {
        let err = parse_flow(b"{not json").unwrap_err();
        assert_eq!(err.reason(), "invalid_json");
        assert_eq!(
            FlowParseError::ProducerError(String::new()).reason(),
            "producer_error"
        );
        assert_eq!(
            FlowParseError::UnsupportedSchemaVersion(2).reason(),
            "unsupported_schema_version"
        );
        assert_eq!(FlowParseError::Validation(vec![]).reason(), "validation");
    }

    #[test]
    fn missing_body_field_is_invalid_json_not_a_panic() {
        let bytes = br#"{"schema_version":1,"title":"x"}"#;
        assert!(matches!(
            parse_flow(bytes),
            Err(FlowParseError::InvalidJson(_))
        ));
    }

    #[test]
    fn validate_collects_every_error() {
        let mut flow = minimal(&["a", "a"], &[("a", "ghost")], &[]);
        flow.processes.push(Process {
            id: "main".into(),
            label: "dup".into(),
        });
        flow.nodes[1].process = Some("nowhere".into());
        let errors = flow.validate().unwrap_err();
        assert!(errors.contains(&FlowValidationError::DuplicateNodeId("a".into())));
        assert!(errors.contains(&FlowValidationError::DuplicateProcessId("main".into())));
        assert!(errors.contains(&FlowValidationError::UnknownProcessRef {
            node: "a".into(),
            process: "nowhere".into()
        }));
        assert!(errors.contains(&FlowValidationError::UnknownNodeRef {
            context: "edge",
            id: "ghost".into()
        }));
        assert!(errors.contains(&FlowValidationError::EmptyEntries));
        assert_eq!(errors.len(), 5);
    }

    #[test]
    fn unknown_entry_and_invalid_ids_are_reported() {
        let mut flow = minimal(&["a", "b;c"], &[], &["a", "zzz"]);
        flow.processes[0].id = " ".into();
        let errors = flow.validate().unwrap_err();
        assert!(errors.contains(&FlowValidationError::InvalidId {
            kind: "process",
            id: " ".into()
        }));
        assert!(errors.contains(&FlowValidationError::InvalidId {
            kind: "node",
            id: "b;c".into()
        }));
        assert!(errors.contains(&FlowValidationError::UnknownNodeRef {
            context: "entries",
            id: "zzz".into()
        }));
    }

    #[test]
    fn backslash_paths_are_normalized_not_rejected() {
        let mut flow = minimal(&["a"], &[], &["a"]);
        flow.nodes[0].location = Some(Location {
            file: "src\\ui\\foo.rs".into(),
            line: 3,
            end_line: Some(9),
        });
        let bytes = serde_json::to_vec(&flow).unwrap();
        let parsed = parse_flow(&bytes).unwrap();
        let location = parsed.nodes[0].location.as_ref().unwrap();
        assert_eq!(location.file, "src/ui/foo.rs");
        assert_eq!(location.display(), "src/ui/foo.rs:3-9");
    }

    #[test]
    fn unsafe_location_paths_are_rejected() {
        for bad in [
            "../x.rs",
            "/etc/passwd",
            "C:/x.rs",
            "//server/share/x",
            "a/../../x",
            "",
        ] {
            let mut flow = minimal(&["a"], &[], &["a"]);
            flow.nodes[0].location = Some(Location {
                file: bad.into(),
                line: 1,
                end_line: None,
            });
            let errors = flow.validate().unwrap_err();
            assert!(
                errors.contains(&FlowValidationError::UnsafeLocationPath {
                    node: "a".into(),
                    file: bad.into()
                }),
                "{bad:?} should be rejected: {errors:?}"
            );
        }
        for good in ["src/main.rs", "./src/main.rs", "a/b.c/d.rs", "x..y/z.rs"] {
            assert!(is_safe_relative_path(good), "{good:?}");
        }
    }

    #[test]
    fn location_display_without_range() {
        let location = Location {
            file: "a.rs".into(),
            line: 7,
            end_line: Some(7),
        };
        assert_eq!(location.display(), "a.rs:7");
    }

    #[test]
    fn resolve_repo_root_relative_and_absolute() {
        let flow_path = Path::new("/flows/x.json");
        assert_eq!(
            resolve_repo_root(flow_path, "repo"),
            PathBuf::from("/flows").join("repo")
        );
        let absolute = if cfg!(windows) {
            "C:\\work\\repo"
        } else {
            "/work/repo"
        };
        assert_eq!(
            resolve_repo_root(flow_path, absolute),
            PathBuf::from(absolute)
        );
        assert_eq!(
            resolve_repo_root(flow_path, "/rooted"),
            PathBuf::from("/rooted")
        );
        assert_eq!(
            resolve_repo_root(Path::new("x.json"), "repo"),
            PathBuf::from("repo")
        );
    }

    #[test]
    fn outgoing_and_incoming_keep_array_order() {
        let flow = minimal(
            &["a", "b", "c"],
            &[("a", "c"), ("a", "b"), ("b", "c")],
            &["a"],
        );
        let out: Vec<usize> = flow.outgoing("a").into_iter().map(|(i, _)| i).collect();
        assert_eq!(out, vec![0, 1]);
        let inc: Vec<&str> = flow
            .incoming("c")
            .into_iter()
            .map(|(_, e)| e.from.as_str())
            .collect();
        assert_eq!(inc, vec!["a", "b"]);
        assert_eq!(flow.process_index("main"), Some(0));
        assert!(flow.process("nope").is_none());
    }

    #[test]
    fn enum_labels_are_stable() {
        assert_eq!(Carrier::ALL.len(), 8);
        assert_eq!(Carrier::InMemory.label(), "in-memory");
        assert_eq!(Carrier::Ui.slug(), "ui");
        assert_eq!(EdgeKind::HandledBy.label(), "handled by");
        assert_eq!(DiffStatus::Added.marker(), Some("+"));
        assert_eq!(DiffStatus::Same.marker(), None);
        assert_eq!(DiffStatus::Modified.slug(), "modified");
        assert_eq!(NodeKind::Event.as_str(), "event");
        assert_eq!(FlowMode::Review.as_str(), "review");
        let json = serde_json::to_string(&EdgeKind::HandledBy).unwrap();
        assert_eq!(json, "\"handled_by\"");
    }

    #[test]
    fn parse_error_display_truncates_long_validation_lists() {
        let errors = (0..5)
            .map(|i| FlowValidationError::DuplicateNodeId(format!("n{i}")))
            .collect();
        let text = FlowParseError::Validation(errors).to_string();
        assert!(text.contains("n2"), "{text}");
        assert!(!text.contains("n3"), "{text}");
        assert!(text.ends_with("(+2 more)"), "{text}");
    }
}
