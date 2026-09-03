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
use super::model::{resolve_repo_root, EdgeKind, Flow, Node, NodeKind};
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
    /// Keyboard cursor inside the last column (index into its items).
    pub column_cursor: Option<usize>,
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
            column_cursor: None,
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

impl FlowPane {
    /// Position of the selected row within `rows` (the current display).
    fn display_index(&self, rows: &[DisplayRow]) -> Option<usize> {
        let selected = self.selected_row?;
        rows.iter().position(|d| d.row == selected)
    }

    /// Select a tree row by index; returns whether the selection changed.
    pub fn select_row(&mut self, row: usize) -> bool {
        if row >= self.rows.len() || self.selected_row == Some(row) {
            return false;
        }
        self.selected_row = Some(row);
        true
    }

    pub fn clear_selection(&mut self) -> bool {
        self.selected_row.take().is_some()
    }

    /// Move the cursor by `delta` display rows, clamped at both ends; with
    /// nothing selected, a downward move lands on the first row and an
    /// upward move on the last.
    pub fn move_selection(&mut self, delta: i64) -> bool {
        let rows = self.display_rows();
        if rows.is_empty() {
            return false;
        }
        let last = rows.len() as i64 - 1;
        let next = match self.display_index(&rows) {
            None if delta >= 0 => 0,
            None => last as usize,
            Some(i) => (i as i64 + delta).clamp(0, last) as usize,
        };
        self.select_row(rows[next].row)
    }

    /// Home / End.
    pub fn select_edge(&mut self, end: bool) -> bool {
        let rows = self.display_rows();
        let Some(target) = (if end { rows.last() } else { rows.first() }) else {
            return false;
        };
        self.select_row(target.row)
    }

    /// Right arrow: expand a collapsed selection, otherwise step into its
    /// first visible child.
    pub fn select_into(&mut self) -> bool {
        let Some(selected) = self.selected_row else {
            return self.move_selection(1);
        };
        if self.collapsed.remove(&selected) {
            return true;
        }
        let rows = self.display_rows();
        let Some(i) = self.display_index(&rows) else {
            return false;
        };
        match rows.get(i + 1) {
            Some(next) if next.depth > rows[i].depth => self.select_row(next.row),
            _ => false,
        }
    }

    /// Left arrow: collapse an expanded selection with children, otherwise
    /// step out to its nearest visible ancestor.
    pub fn select_out(&mut self) -> bool {
        let Some(selected) = self.selected_row else {
            return false;
        };
        if self.rows[selected].child_count > 0 && !self.collapsed.contains(&selected) {
            self.collapsed.insert(selected);
            return true;
        }
        let rows = self.display_rows();
        let Some(i) = self.display_index(&rows) else {
            return false;
        };
        let depth = rows[i].depth;
        if depth == 0 {
            return false;
        }
        match rows[..i].iter().rev().find(|d| d.depth < depth) {
            Some(parent) => self.select_row(parent.row),
            None => false,
        }
    }

    /// After a collapse or level change the selection may sit on a hidden
    /// row; move it to the nearest visible ancestor (or the first row).
    pub fn clamp_selection(&mut self) -> bool {
        let Some(selected) = self.selected_row else {
            return false;
        };
        let rows = self.display_rows();
        let visible = |row: usize| rows.iter().any(|d| d.row == row);
        if visible(selected) {
            return false;
        }
        let mut cursor = self.rows[selected].parent;
        while let Some(parent) = cursor {
            if visible(parent) {
                self.selected_row = Some(parent);
                return true;
            }
            cursor = self.rows[parent].parent;
        }
        self.selected_row = rows.first().map(|d| d.row);
        true
    }

    /// Node id of the selected row, for `src` and column handoffs.
    pub fn selected_node_id(&self) -> Option<&str> {
        self.selected_row
            .and_then(|row| self.rows.get(row))
            .map(|r| r.node_id.as_str())
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;
    use crate::flow_explorer::test_support::fixture_path;

    fn pane() -> FlowPane {
        FlowPane::open(&fixture_path()).unwrap()
    }

    #[test]
    fn move_selection_clamps_and_starts_at_the_edges() {
        let mut p = pane();
        assert!(p.move_selection(0), "no selection: lands on the first row");
        assert_eq!(p.selected_row, Some(0));
        assert!(p.move_selection(3));
        assert_eq!(p.selected_row, Some(3));
        assert!(p.move_selection(100));
        assert_eq!(p.selected_row, Some(10));
        assert!(!p.move_selection(1));
        assert!(p.move_selection(-100));
        assert_eq!(p.selected_row, Some(0));
        p.clear_selection();
        assert!(p.move_selection(-1));
        assert_eq!(p.selected_row, Some(10));
        assert!(!p.select_row(99));
        assert!(p.select_edge(false));
        assert_eq!(p.selected_row, Some(0));
        assert!(p.select_edge(true));
        assert_eq!(p.selected_row, Some(10));
    }

    #[test]
    fn move_selection_skips_rows_hidden_by_collapse_and_level() {
        let mut p = pane();
        assert!(p.toggle_collapsed(3));
        p.select_row(3);
        assert!(
            !p.move_selection(1),
            "nothing visible below a collapsed row 3"
        );
        assert_eq!(p.selected_row, Some(3));
        p.expand_all();
        p.set_level(FlowLevel::Events);
        p.select_row(1);
        assert!(p.move_selection(1));
        assert_eq!(
            p.selected_row,
            Some(3),
            "row 2 is hidden at the events level"
        );
    }

    #[test]
    fn select_into_expands_then_steps_into_the_child() {
        let mut p = pane();
        assert!(p.select_into(), "no selection: moves onto the first row");
        assert_eq!(p.selected_row, Some(0));
        assert!(p.select_into());
        assert_eq!(p.selected_row, Some(1));
        p.toggle_collapsed(1);
        assert!(p.select_into(), "expands the collapsed row first");
        assert!(!p.collapsed.contains(&1));
        assert_eq!(p.selected_row, Some(1));
        assert!(p.select_into());
        assert_eq!(p.selected_row, Some(2));
        p.select_row(10);
        assert!(!p.select_into(), "leaf: nothing to step into");
    }

    #[test]
    fn select_out_collapses_then_steps_to_the_parent() {
        let mut p = pane();
        assert!(!p.select_out());
        p.select_row(7);
        assert!(p.select_out(), "leaf: moves to the parent");
        assert_eq!(p.selected_row, Some(6));
        assert!(p.select_out(), "expanded row with children collapses");
        assert!(p.collapsed.contains(&6));
        assert_eq!(p.selected_row, Some(6));
        assert!(p.select_out(), "collapsed row: moves to the parent");
        assert_eq!(p.selected_row, Some(5));
        p.select_row(0);
        assert!(p.select_out());
        assert!(p.collapsed.contains(&0));
        assert!(!p.select_out(), "entry row has no parent");
    }

    #[test]
    fn clamp_selection_moves_a_hidden_selection_to_its_visible_ancestor() {
        let mut p = pane();
        p.select_row(8);
        assert!(!p.clamp_selection());
        p.toggle_collapsed(4);
        assert!(p.clamp_selection());
        assert_eq!(p.selected_row, Some(4));
        p.expand_all();
        p.select_row(2);
        p.set_level(FlowLevel::Events);
        assert!(p.clamp_selection());
        assert_eq!(
            p.selected_row,
            Some(1),
            "submit is hidden; handleKeyDown is its visible parent"
        );
        assert_eq!(p.selected_node_id(), Some("Editor.tsx::handleKeyDown"));
    }
}

/// Where an item in a Miller column comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnSection {
    /// Column 0: the flow's entry events.
    Entries,
    /// Outgoing `calls` edges.
    Calls,
    /// Outgoing `resolves` edges.
    Resolves,
    /// Outgoing `handled_by` edges (an event's handlers).
    HandledBy,
    /// Incoming `handled_by` edges (the events a function handles).
    Handles,
}

impl ColumnSection {
    pub fn label(self) -> &'static str {
        match self {
            ColumnSection::Entries => "entries",
            ColumnSection::Calls => "calls",
            ColumnSection::Resolves => "resolves",
            ColumnSection::HandledBy => "handled by",
            ColumnSection::Handles => "handles",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnItem {
    pub section: ColumnSection,
    pub node_id: String,
}

impl FlowPane {
    /// Columns on screen: the overview plus one per focused node.
    pub fn column_count(&self) -> usize {
        self.path.len() + 1
    }

    /// The node column `col` describes (`None` for the overview column).
    pub fn column_node(&self, col: usize) -> Option<&Node> {
        let id = self.path.get(col.checked_sub(1)?)?;
        self.flow.node(id)
    }

    /// Clickable items of column `col`, in section order: outgoing edges in
    /// array order, then the events this node handles.
    pub fn column_items(&self, col: usize) -> Vec<ColumnItem> {
        if col == 0 {
            return self
                .flow
                .entries
                .iter()
                .filter(|id| self.flow.node(id).is_some())
                .map(|id| ColumnItem {
                    section: ColumnSection::Entries,
                    node_id: id.clone(),
                })
                .collect();
        }
        let Some(node) = self.column_node(col) else {
            return Vec::new();
        };
        let mut items = Vec::new();
        for (_, edge) in self.flow.outgoing(&node.id) {
            if self.flow.node(&edge.to).is_none() {
                continue;
            }
            let section = match edge.kind {
                EdgeKind::Calls => ColumnSection::Calls,
                EdgeKind::Resolves => ColumnSection::Resolves,
                EdgeKind::HandledBy => ColumnSection::HandledBy,
            };
            items.push(ColumnItem {
                section,
                node_id: edge.to.clone(),
            });
        }
        for (_, edge) in self.flow.incoming(&node.id) {
            if edge.kind == EdgeKind::HandledBy && self.flow.node(&edge.from).is_some() {
                items.push(ColumnItem {
                    section: ColumnSection::Handles,
                    node_id: edge.from.clone(),
                });
            }
        }
        items
    }

    /// `flow.select:<col>:<id>`: focus `id`, which must be an item of column
    /// `col`; deeper columns are dropped.
    pub fn select_column(&mut self, col: usize, node_id: &str) -> bool {
        if col > self.path.len() {
            return false;
        }
        if !self.column_items(col).iter().any(|i| i.node_id == node_id) {
            return false;
        }
        self.path.truncate(col);
        self.path.push(node_id.to_string());
        self.column_cursor = None;
        true
    }

    /// `flow.focus:<col>`: make column `col` the last one (a click on a
    /// collapsed strip).
    pub fn focus_column(&mut self, col: usize) -> bool {
        if col >= self.column_count() || col == self.path.len() {
            return false;
        }
        self.path.truncate(col);
        self.column_cursor = None;
        true
    }

    /// Items of the last (focused) column.
    pub fn focused_items(&self) -> Vec<ColumnItem> {
        self.column_items(self.path.len())
    }

    /// Up/Down inside the focused column.
    pub fn column_move(&mut self, delta: i64) -> bool {
        let items = self.focused_items();
        if items.is_empty() {
            return false;
        }
        let last = items.len() as i64 - 1;
        let next = match self.column_cursor {
            None if delta >= 0 => 0,
            None => last as usize,
            Some(i) => (i as i64 + delta).clamp(0, last) as usize,
        };
        if self.column_cursor == Some(next) {
            return false;
        }
        self.column_cursor = Some(next);
        true
    }

    /// Enter/Right: open the item under the cursor (or the first item).
    pub fn column_enter(&mut self) -> bool {
        let items = self.focused_items();
        let index = self.column_cursor.unwrap_or(0);
        let Some(item) = items.get(index) else {
            return false;
        };
        let col = self.path.len();
        let id = item.node_id.clone();
        self.select_column(col, &id)
    }

    /// Left: drop the focused column; the cursor lands on the node that
    /// column described so Right reopens it.
    pub fn column_back(&mut self) -> bool {
        let Some(last) = self.path.pop() else {
            return false;
        };
        let items = self.focused_items();
        self.column_cursor = items.iter().position(|i| i.node_id == last);
        true
    }

    /// The node the panes view is "on": the cursor item if any, else the
    /// last focused column's node.
    pub fn panes_focus_node_id(&self) -> Option<String> {
        if let Some(index) = self.column_cursor {
            if let Some(item) = self.focused_items().get(index) {
                return Some(item.node_id.clone());
            }
        }
        self.path.last().cloned()
    }
}

#[cfg(test)]
mod column_tests {
    use super::*;
    use crate::flow_explorer::test_support::fixture_path;

    fn pane() -> FlowPane {
        FlowPane::open(&fixture_path()).unwrap()
    }

    fn ids(items: &[ColumnItem]) -> Vec<&str> {
        items.iter().map(|i| i.node_id.as_str()).collect()
    }

    #[test]
    fn overview_column_lists_entries() {
        let p = pane();
        assert_eq!(p.column_count(), 1);
        assert!(p.column_node(0).is_none());
        let items = p.column_items(0);
        assert_eq!(ids(&items), vec!["ui.cmd-enter"]);
        assert_eq!(items[0].section, ColumnSection::Entries);
        assert!(p.column_items(1).is_empty());
    }

    #[test]
    fn select_column_builds_the_path_and_lists_edges_by_section() {
        let mut p = pane();
        assert!(
            !p.select_column(0, "Editor.tsx::handleKeyDown"),
            "not an entry"
        );
        assert!(
            !p.select_column(1, "ui.cmd-enter"),
            "column 1 does not exist yet"
        );
        assert!(p.select_column(0, "ui.cmd-enter"));
        assert_eq!(p.path, vec!["ui.cmd-enter"]);
        let items = p.column_items(1);
        assert_eq!(ids(&items), vec!["Editor.tsx::handleKeyDown"]);
        assert_eq!(items[0].section, ColumnSection::HandledBy);

        assert!(p.select_column(1, "Editor.tsx::handleKeyDown"));
        let items = p.column_items(2);
        assert_eq!(
            ids(&items),
            vec!["AgentPane.tsx::submit", "ui.cmd-enter"],
            "calls first, then the event it handles"
        );
        assert_eq!(items[0].section, ColumnSection::Calls);
        assert_eq!(items[1].section, ColumnSection::Handles);

        // Re-selecting from an earlier column drops the deeper ones.
        assert!(p.select_column(0, "ui.cmd-enter"));
        assert_eq!(p.path, vec!["ui.cmd-enter"]);
        assert_eq!(p.column_count(), 2);
    }

    #[test]
    fn resolves_edges_get_their_own_section() {
        let mut p = pane();
        p.path = vec!["HaloAgentSession.ts::prompt".into()];
        let items = p.column_items(1);
        assert_eq!(ids(&items), vec!["rpc.sessions.prompt.resolves"]);
        assert_eq!(items[0].section, ColumnSection::Resolves);
    }

    #[test]
    fn focus_column_truncates() {
        let mut p = pane();
        p.path = vec![
            "ui.cmd-enter".into(),
            "Editor.tsx::handleKeyDown".into(),
            "AgentPane.tsx::submit".into(),
        ];
        assert!(!p.focus_column(3), "already the last column");
        assert!(!p.focus_column(9));
        assert!(p.focus_column(1));
        assert_eq!(p.path, vec!["ui.cmd-enter"]);
        assert!(p.focus_column(0));
        assert!(p.path.is_empty());
    }

    #[test]
    fn column_cursor_moves_enters_and_backs_out() {
        let mut p = pane();
        assert!(p.column_move(1));
        assert_eq!(p.column_cursor, Some(0));
        assert!(!p.column_move(1), "one entry: clamped");
        assert!(p.column_enter());
        assert_eq!(p.path, vec!["ui.cmd-enter"]);
        assert_eq!(p.column_cursor, None);
        assert!(p.column_enter(), "no cursor: opens the first item");
        assert_eq!(p.path, vec!["ui.cmd-enter", "Editor.tsx::handleKeyDown"]);
        assert!(p.column_move(1));
        assert!(p.column_move(1));
        assert_eq!(p.column_cursor, Some(1), "handles section item");
        assert_eq!(p.panes_focus_node_id().as_deref(), Some("ui.cmd-enter"));
        assert!(p.column_back());
        assert_eq!(p.path, vec!["ui.cmd-enter"]);
        assert_eq!(p.column_cursor, Some(0), "cursor on the column just closed");
        assert!(p.column_back());
        assert!(p.path.is_empty());
        assert!(!p.column_back());
        assert_eq!(p.panes_focus_node_id().as_deref(), Some("ui.cmd-enter"));
    }
}

impl FlowPane {
    /// Home / End inside the focused column.
    pub fn column_edge(&mut self, end: bool) -> bool {
        let items = self.focused_items();
        if items.is_empty() {
            return false;
        }
        let target = if end { items.len() - 1 } else { 0 };
        if self.column_cursor == Some(target) {
            return false;
        }
        self.column_cursor = Some(target);
        true
    }

    /// Escape: drop the column cursor.
    pub fn column_clear(&mut self) -> bool {
        self.column_cursor.take().is_some()
    }
}

#[cfg(test)]
mod column_edge_tests {
    use super::*;
    use crate::flow_explorer::test_support::fixture_path;

    #[test]
    fn column_edge_and_clear() {
        let mut p = FlowPane::open(&fixture_path()).unwrap();
        p.path = vec!["ui.cmd-enter".into(), "Editor.tsx::handleKeyDown".into()];
        assert!(p.column_edge(true));
        assert_eq!(p.column_cursor, Some(1));
        assert!(!p.column_edge(true));
        assert!(p.column_edge(false));
        assert_eq!(p.column_cursor, Some(0));
        assert!(p.column_clear());
        assert!(!p.column_clear());
        p.path = vec!["SessionRegistry.ts::open".into()];
        assert!(
            p.column_items(1).is_empty(),
            "a leaf called by a function lists nothing"
        );
        assert!(!p.column_edge(false));
    }
}
