//! Per-pane state for an open flow: the immutable document plus the view
//! state every render reads.
//!
//! Lives in `AppState.flows` behind an `Arc` so the per-frame snapshot
//! clones a pointer rather than the document; mutations go through
//! `Arc::make_mut` on the dispatch path, which only copies when a snapshot
//! still holds the previous version.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::ingest::{ingest_file, IngestError};
use super::model::{resolve_repo_root, Flow, NodeKind};
use super::snippet::{load_snippet, Snippet, SnippetError, CONTEXT_LINES};
use super::tree::{collapsible_rows, derive_tree, visible_rows, TreeRow};

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
    /// Loaded source excerpts (and failures) keyed by node id, filled on
    /// the dispatch path so render never touches the disk.
    pub snippets: HashMap<String, Result<Snippet, SnippetError>>,
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
            snippets: HashMap::new(),
            opened_unix_ms: crate::telemetry_sink::now_unix_ms(),
        }
    }

    pub fn title(&self) -> &str {
        &self.flow.title
    }
}

/// One call-stack row as rendered: the tree row index plus the depth the
/// current level shows it at (the Events level hides function rows, so
/// display depth can be shallower than the tree depth).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayRow {
    pub row: usize,
    pub depth: usize,
}

impl FlowPane {
    /// Returns `true` when the view actually changed.
    pub fn set_view(&mut self, view: FlowView) -> bool {
        if self.view == view {
            return false;
        }
        self.view = view;
        true
    }

    /// `Source` opens every snippet; the other levels close them so the
    /// level acts as a three-way switch rather than accumulating state.
    pub fn set_level(&mut self, level: FlowLevel) {
        self.level = level;
        self.src_open.clear();
        if level == FlowLevel::Source {
            self.src_open.extend(
                self.flow
                    .nodes
                    .iter()
                    .filter(|n| n.location.is_some())
                    .map(|n| n.id.clone()),
            );
        }
    }

    /// Collapse or expand a row's children. Leaves are ignored; returns
    /// whether anything changed.
    pub fn toggle_collapsed(&mut self, row: usize) -> bool {
        match self.rows.get(row) {
            Some(r) if r.child_count > 0 => {
                if !self.collapsed.remove(&row) {
                    self.collapsed.insert(row);
                }
                true
            }
            _ => false,
        }
    }

    pub fn expand_all(&mut self) {
        self.collapsed.clear();
    }

    pub fn collapse_all(&mut self) {
        self.collapsed = collapsible_rows(&self.rows);
    }

    /// Open or close a node's inline source. Nodes without a location are
    /// ignored; returns the new open state when something changed.
    pub fn toggle_src(&mut self, node_id: &str) -> Option<bool> {
        let has_location = self.flow.node(node_id)?.location.is_some();
        if !has_location {
            return None;
        }
        if self.src_open.remove(node_id) {
            Some(false)
        } else {
            self.src_open.insert(node_id.to_string());
            Some(true)
        }
    }

    /// Load the node's source excerpt if it is not cached yet. `None` when
    /// the node has no location; otherwise whether a load happened now (the
    /// caller reports fresh unexpected failures).
    pub fn ensure_snippet(&mut self, node_id: &str) -> Option<bool> {
        if self.snippets.contains_key(node_id) {
            return Some(false);
        }
        let location = self.flow.node(node_id)?.location.clone()?;
        let result = load_snippet(&self.repo_root, &location, CONTEXT_LINES);
        self.snippets.insert(node_id.to_string(), result);
        Some(true)
    }

    /// The cached excerpt for a node, if any load was attempted.
    pub fn snippet(&self, node_id: &str) -> Option<&Result<Snippet, SnippetError>> {
        self.snippets.get(node_id)
    }

    /// Node ids whose snippet is open but not loaded yet.
    pub fn unloaded_open_snippets(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .src_open
            .iter()
            .filter(|id| !self.snippets.contains_key(*id))
            .cloned()
            .collect();
        ids.sort();
        ids
    }

    /// Rows the call stack shows right now, honouring collapse state and
    /// the level filter. `Events` keeps event rows, the row each event
    /// hangs off (its emitter) and each event's direct children (its
    /// handlers), so every hop still reads as "who sent it, who got it".
    pub fn display_rows(&self) -> Vec<DisplayRow> {
        let visible = visible_rows(&self.rows, &self.collapsed);
        if self.level != FlowLevel::Events {
            return visible
                .into_iter()
                .map(|row| DisplayRow {
                    row,
                    depth: self.rows[row].depth,
                })
                .collect();
        }

        let is_event = |row: usize| {
            self.flow
                .node(&self.rows[row].node_id)
                .is_some_and(|n| n.kind == NodeKind::Event)
        };
        let mut keep = vec![false; self.rows.len()];
        for (i, row) in self.rows.iter().enumerate() {
            if !is_event(i) {
                continue;
            }
            keep[i] = true;
            if let Some(parent) = row.parent {
                keep[parent] = true;
            }
        }
        for (i, row) in self.rows.iter().enumerate() {
            if row.parent.is_some_and(|p| is_event(p)) {
                keep[i] = true;
            }
        }

        let mut out = Vec::new();
        for row in visible {
            if !keep[row] {
                continue;
            }
            let mut depth = 0;
            let mut cursor = self.rows[row].parent;
            while let Some(p) = cursor {
                if keep[p] {
                    depth += 1;
                }
                cursor = self.rows[p].parent;
            }
            out.push(DisplayRow { row, depth });
        }
        out
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

#[cfg(test)]
mod transition_tests {
    use super::*;
    use crate::flow_explorer::test_support::fixture_path;

    fn pane() -> FlowPane {
        FlowPane::open(&fixture_path()).unwrap()
    }

    #[test]
    fn set_view_reports_change() {
        let mut p = pane();
        assert!(!p.set_view(FlowView::CallStack));
        assert!(p.set_view(FlowView::Graph));
        assert_eq!(p.view, FlowView::Graph);
        assert!(!p.set_view(FlowView::Graph));
    }

    #[test]
    fn source_level_opens_every_located_node_and_other_levels_close_them() {
        let mut p = pane();
        p.set_level(FlowLevel::Source);
        let located = p.flow.nodes.iter().filter(|n| n.location.is_some()).count();
        assert_eq!(p.src_open.len(), located);
        assert!(located > 0);
        p.set_level(FlowLevel::Code);
        assert!(p.src_open.is_empty());
        assert_eq!(p.level, FlowLevel::Code);
    }

    #[test]
    fn toggle_collapsed_ignores_leaves_and_round_trips() {
        let mut p = pane();
        let leaf = p.rows.len() - 1;
        assert_eq!(p.rows[leaf].child_count, 0);
        assert!(!p.toggle_collapsed(leaf));
        assert!(!p.toggle_collapsed(usize::MAX));
        assert!(p.toggle_collapsed(0));
        assert!(p.collapsed.contains(&0));
        assert_eq!(p.display_rows().len(), 1);
        assert!(p.toggle_collapsed(0));
        assert_eq!(p.display_rows().len(), p.rows.len());
    }

    #[test]
    fn collapse_all_then_expand_all() {
        let mut p = pane();
        p.collapse_all();
        assert_eq!(p.collapsed.len(), 9);
        assert_eq!(p.display_rows().len(), 1);
        p.expand_all();
        assert!(p.collapsed.is_empty());
        assert_eq!(p.display_rows().len(), 11);
    }

    #[test]
    fn toggle_src_only_for_located_nodes() {
        let mut p = pane();
        assert_eq!(p.toggle_src("ui.cmd-enter"), None);
        assert_eq!(p.toggle_src("nope"), None);
        assert_eq!(p.toggle_src("Editor.tsx::handleKeyDown"), Some(true));
        assert!(p.src_open.contains("Editor.tsx::handleKeyDown"));
        assert_eq!(p.toggle_src("Editor.tsx::handleKeyDown"), Some(false));
        assert!(p.src_open.is_empty());
    }

    #[test]
    fn code_level_shows_tree_depths_verbatim() {
        let p = pane();
        let rows = p.display_rows();
        assert_eq!(rows.len(), 11);
        for d in &rows {
            assert_eq!(d.depth, p.rows[d.row].depth);
        }
    }

    #[test]
    fn events_level_keeps_events_their_emitters_and_handlers() {
        let mut p = pane();
        p.set_level(FlowLevel::Events);
        let rows = p.display_rows();
        let ids: Vec<&str> = rows
            .iter()
            .map(|d| p.rows[d.row].node_id.as_str())
            .collect();
        // Fixture chain: ui.cmd-enter (event, entry) -> handleKeyDown ->
        // submit -> useAgentSession.prompt -> rpc.sessions.prompt (event)
        // -> RPCHandler.upgrade -> ... -> HaloAgentSession.prompt ->
        // rpc.sessions.prompt.resolves (event) -> invalidateQueries.
        assert_eq!(
            ids,
            vec![
                "ui.cmd-enter",
                "Editor.tsx::handleKeyDown",
                "useAgentSession.ts::prompt",
                "rpc.sessions.prompt",
                "main.ts::RPCHandler.upgrade",
                "HaloAgentSession.ts::prompt",
                "rpc.sessions.prompt.resolves",
                "useAgentSession.ts::invalidateQueries",
            ]
        );
        let depths: Vec<usize> = rows.iter().map(|d| d.depth).collect();
        assert_eq!(depths, vec![0, 1, 2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn events_level_still_honours_collapse() {
        let mut p = pane();
        p.set_level(FlowLevel::Events);
        // Collapse the useAgentSession.prompt row: everything under it goes.
        let row = p
            .rows
            .iter()
            .position(|r| r.node_id == "useAgentSession.ts::prompt")
            .unwrap();
        assert!(p.toggle_collapsed(row));
        let ids: Vec<&str> = p
            .display_rows()
            .iter()
            .map(|d| p.rows[d.row].node_id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec![
                "ui.cmd-enter",
                "Editor.tsx::handleKeyDown",
                "useAgentSession.ts::prompt"
            ]
        );
    }

    #[test]
    fn ensure_snippet_loads_once_and_caches_failures() {
        let mut p = pane();
        assert_eq!(p.ensure_snippet("ui.cmd-enter"), None);
        assert_eq!(p.ensure_snippet("nope"), None);
        assert_eq!(p.ensure_snippet("SessionRegistry.ts::open"), Some(true));
        assert_eq!(p.ensure_snippet("SessionRegistry.ts::open"), Some(false));
        let snippet = p
            .snippet("SessionRegistry.ts::open")
            .unwrap()
            .as_ref()
            .unwrap();
        assert_eq!(snippet.hl_start, 31);
        assert_eq!(snippet.hl_end, 41);
        // A missing file is cached as its error and not retried.
        p.flow.nodes[1].location.as_mut().unwrap().file = "gone/Missing.tsx".into();
        assert_eq!(p.ensure_snippet("Editor.tsx::handleKeyDown"), Some(true));
        assert!(matches!(
            p.snippet("Editor.tsx::handleKeyDown"),
            Some(Err(SnippetError::NotFound))
        ));
        assert_eq!(p.ensure_snippet("Editor.tsx::handleKeyDown"), Some(false));
    }

    #[test]
    fn unloaded_open_snippets_lists_open_ids_without_a_cache_entry() {
        let mut p = pane();
        p.set_level(FlowLevel::Source);
        assert_eq!(p.unloaded_open_snippets().len(), 8);
        p.ensure_snippet("SessionRegistry.ts::open");
        let ids = p.unloaded_open_snippets();
        assert_eq!(ids.len(), 7);
        assert!(!ids.iter().any(|id| id == "SessionRegistry.ts::open"));
    }
}
