//! Per-pane state for an open flow: the immutable document plus the view
//! state every render reads.
//!
//! Lives in `AppState.flows` behind an `Arc` so the per-frame snapshot
//! clones a pointer rather than the document; mutations go through
//! `Arc::make_mut` on the dispatch path, which only copies when a snapshot
//! still holds the previous version.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::ingest::{ingest_file, IngestError};
use super::model::{resolve_repo_root, Flow};
use super::tree::{derive_tree, TreeRow};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlowView {
    #[default]
    CallStack,
    Panes,
    Graph,
}

impl FlowView {
    /// Name used in dispatch strings (`flow.view:<name>`) and telemetry.
    pub fn as_str(self) -> &'static str {
        match self {
            FlowView::CallStack => "stack",
            FlowView::Panes => "panes",
            FlowView::Graph => "graph",
        }
    }

    pub fn parse(name: &str) -> Option<FlowView> {
        match name {
            "stack" => Some(FlowView::CallStack),
            "panes" => Some(FlowView::Panes),
            "graph" => Some(FlowView::Graph),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlowLevel {
    /// Only events and the functions that emit or handle them.
    Events,
    /// Every node.
    #[default]
    Code,
    /// Every node with its source snippet open.
    Source,
}

impl FlowLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            FlowLevel::Events => "events",
            FlowLevel::Code => "code",
            FlowLevel::Source => "source",
        }
    }

    pub fn parse(name: &str) -> Option<FlowLevel> {
        match name {
            "events" => Some(FlowLevel::Events),
            "code" => Some(FlowLevel::Code),
            "source" => Some(FlowLevel::Source),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FlowPane {
    pub flow: Flow,
    /// The JSON file this pane was opened from.
    pub source_path: PathBuf,
    /// `flow.repo_root` resolved against `source_path`.
    pub repo_root: PathBuf,
    /// Telemetry correlation id: the source file's stem.
    pub flow_id: String,
    /// Fully expanded call-stack rows, derived once at open.
    pub rows: Vec<TreeRow>,
    pub view: FlowView,
    pub level: FlowLevel,
    /// Row indices whose descendants are hidden in the call stack.
    pub collapsed: HashSet<usize>,
    /// Node ids whose source snippet is open in the call stack.
    pub src_open: HashSet<String>,
    /// Keyboard cursor in the call stack (row index).
    pub selected_row: Option<usize>,
    /// Miller-column focus chain: column `c + 1` lists the children of
    /// `path[c]`.
    pub path: Vec<String>,
    pub opened_unix_ms: u64,
}

impl FlowPane {
    /// Read, validate and derive; the single entry point for both the
    /// manual open and the launch poller.
    pub fn open(path: &Path) -> Result<FlowPane, IngestError> {
        let flow = ingest_file(path)?;
        Ok(Self::from_flow(flow, path))
    }

    pub fn from_flow(flow: Flow, source_path: &Path) -> FlowPane {
        let rows = derive_tree(&flow);
        let repo_root = resolve_repo_root(source_path, &flow.repo_root);
        FlowPane {
            flow_id: flow_id_for(source_path),
            rows,
            repo_root,
            flow,
            source_path: source_path.to_path_buf(),
            view: FlowView::default(),
            level: FlowLevel::default(),
            collapsed: HashSet::new(),
            src_open: HashSet::new(),
            selected_row: None,
            path: Vec::new(),
            opened_unix_ms: crate::telemetry_sink::now_unix_ms(),
        }
    }

    pub fn title(&self) -> &str {
        &self.flow.title
    }
}

/// The file stem (`<flow_id>.json` as written by the launcher, or whatever
/// the user picked), never node content.
pub fn flow_id_for(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "flow".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_explorer::test_support::fixture_path;

    #[test]
    fn open_derives_rows_and_resolves_the_repo_root() {
        let pane = FlowPane::open(&fixture_path()).unwrap();
        assert_eq!(pane.title(), "Send a prompt");
        assert_eq!(pane.rows.len(), 11);
        assert_eq!(pane.flow_id, "send-a-prompt");
        assert!(pane.repo_root.is_dir(), "{}", pane.repo_root.display());
        assert_eq!(pane.view, FlowView::CallStack);
        assert_eq!(pane.level, FlowLevel::Code);
        assert!(pane.collapsed.is_empty());
        assert!(pane.path.is_empty());
        assert!(pane.opened_unix_ms > 0);
    }

    #[test]
    fn open_failure_keeps_the_ingest_reason() {
        let err = FlowPane::open(Path::new("C:/nope/flow.json")).unwrap_err();
        assert_eq!(err.reason(), "not_found");
    }

    #[test]
    fn view_and_level_names_round_trip() {
        for view in [FlowView::CallStack, FlowView::Panes, FlowView::Graph] {
            assert_eq!(FlowView::parse(view.as_str()), Some(view));
        }
        for level in [FlowLevel::Events, FlowLevel::Code, FlowLevel::Source] {
            assert_eq!(FlowLevel::parse(level.as_str()), Some(level));
        }
        assert_eq!(FlowView::parse("tree"), None);
        assert_eq!(FlowLevel::parse(""), None);
    }

    #[test]
    fn flow_id_is_the_file_stem() {
        assert_eq!(flow_id_for(Path::new("C:/x/abc-123.json")), "abc-123");
        assert_eq!(flow_id_for(Path::new("")), "flow");
    }
}
