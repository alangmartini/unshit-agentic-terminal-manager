//! Flow Explorer: an agent-authored model of one user-facing flow through a
//! codebase (events, handlers, IPC hops, state), rendered as a navigable
//! call-stack tree, Miller columns and a swim-lane graph.
//!
//! The data model lives in [`model`]; every view derives its structure from
//! the flow's ordered edges ([`tree`]). Nothing in this module touches the
//! render path or spawns processes; producers and ingestion are layered on
//! top by the app state.

pub mod highlight;
pub mod ingest;
pub mod model;
pub mod pane;
pub mod snippet;
pub mod telemetry;
pub mod tree;

pub use highlight::{tokenize, Language, Token, TokenKind};
pub use ingest::{ingest_file, IngestError, MAX_FLOW_JSON_BYTES};
pub use model::{
    parse_flow, resolve_repo_root, Carrier, DiffRange, DiffStatus, Edge, EdgeKind, Flow, FlowMode,
    FlowParseError, FlowValidationError, Location, Node, NodeKind, Process, FLOW_SCHEMA_VERSION,
};
pub use pane::{flow_id_for, ColumnItem, ColumnSection, DisplayRow, FlowLevel, FlowPane, FlowView};
pub use snippet::{load_snippet, Snippet, SnippetError, CONTEXT_LINES, MAX_SNIPPET_BYTES};
pub use tree::{
    collapsible_rows, derive_tree, visible_rows, RowMarker, TreeRow, MAX_TREE_DEPTH, MAX_TREE_ROWS,
};

/// Where app-launched flows are written and where the manual picker
/// starts: `<data_dir>/flows/`.
pub fn flows_dir() -> Option<std::path::PathBuf> {
    crate::profile::data_dir().map(|dir| dir.join("flows"))
}

/// Shared fixture helpers for unit tests across the module and the UI.
#[cfg(test)]
pub mod test_support {
    use std::path::{Path, PathBuf};

    use super::model::{parse_flow, Flow};

    /// The video transcription fixture ("Send a prompt", 11 nodes).
    pub fn fixture_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/flow-explorer/send-a-prompt.json")
    }

    pub fn load_fixture() -> Flow {
        let bytes = std::fs::read(fixture_path()).expect("fixture readable");
        parse_flow(&bytes).expect("fixture parses and validates")
    }
}
