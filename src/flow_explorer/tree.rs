//! Call-stack tree derived from a flow's ordered edges.
//!
//! Pure: a depth-first walk from `entries` following outgoing edges in array
//! order. A node already on the current path is emitted once more as a leaf
//! that points back at its earlier row, so cycles cannot loop, and both a
//! depth cap and a total-row cap bound a hostile document. Rows are computed
//! once at ingest and cached on the pane; views filter them per frame.

use std::collections::{HashMap, HashSet};

use super::model::{EdgeKind, Flow};

/// Deeper than this and the walk stops expanding (a `Truncated` row).
pub const MAX_TREE_DEPTH: usize = 64;
/// More rows than this and the walk stops entirely (one `Truncated` row).
pub const MAX_TREE_ROWS: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowMarker {
    /// Ordinary row; its children (if any) follow it.
    None,
    /// The node is already on the current path (a cycle); rendered as a
    /// leaf pointing at the earlier row.
    Repeat { row: usize },
    /// Traversal stopped here (depth or row cap); children are not shown.
    Truncated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    pub node_id: String,
    pub depth: usize,
    /// Kind of the edge that led here; `None` for entries.
    pub edge_kind: Option<EdgeKind>,
    /// Row index of the parent; `None` for entries.
    pub parent: Option<usize>,
    /// Last among its siblings (drives the `└─` connector).
    pub is_last_sibling: bool,
    /// Children emitted directly under this row.
    pub child_count: usize,
    pub marker: RowMarker,
}

/// Every row of the fully expanded tree, in display order.
pub fn derive_tree(flow: &Flow) -> Vec<TreeRow> {
    let known: HashSet<&str> = flow.nodes.iter().map(|n| n.id.as_str()).collect();
    let mut children: HashMap<&str, Vec<(&str, EdgeKind)>> = HashMap::new();
    for edge in &flow.edges {
        if known.contains(edge.from.as_str()) && known.contains(edge.to.as_str()) {
            children
                .entry(edge.from.as_str())
                .or_default()
                .push((edge.to.as_str(), edge.kind));
        }
    }
    let entries: Vec<&str> = flow
        .entries
        .iter()
        .map(String::as_str)
        .filter(|id| known.contains(id))
        .collect();
    let mut builder = Builder {
        children,
        rows: Vec::new(),
        path: Vec::new(),
        truncated: false,
    };
    let last = entries.len().saturating_sub(1);
    for (i, entry) in entries.iter().copied().enumerate() {
        builder.visit(entry, 0, None, None, i == last);
    }
    builder.rows
}

struct Builder<'a> {
    /// Outgoing edges per node, array order, dangling refs removed.
    children: HashMap<&'a str, Vec<(&'a str, EdgeKind)>>,
    rows: Vec<TreeRow>,
    /// (node id, row index) for every ancestor of the node being visited.
    path: Vec<(&'a str, usize)>,
    truncated: bool,
}

impl<'a> Builder<'a> {
    fn visit(
        &mut self,
        id: &'a str,
        depth: usize,
        edge_kind: Option<EdgeKind>,
        parent: Option<usize>,
        is_last_sibling: bool,
    ) {
        if self.rows.len() >= MAX_TREE_ROWS {
            if !self.truncated {
                self.truncated = true;
                self.rows.push(TreeRow {
                    node_id: id.to_string(),
                    depth,
                    edge_kind,
                    parent,
                    is_last_sibling: true,
                    child_count: 0,
                    marker: RowMarker::Truncated,
                });
            }
            return;
        }
        let row = self.rows.len();
        if let Some(&(_, earlier)) = self.path.iter().find(|(node, _)| *node == id) {
            self.rows.push(TreeRow {
                node_id: id.to_string(),
                depth,
                edge_kind,
                parent,
                is_last_sibling,
                child_count: 0,
                marker: RowMarker::Repeat { row: earlier },
            });
            return;
        }
        let kids: Vec<(&'a str, EdgeKind)> = self.children.get(id).cloned().unwrap_or_default();
        if depth >= MAX_TREE_DEPTH && !kids.is_empty() {
            self.rows.push(TreeRow {
                node_id: id.to_string(),
                depth,
                edge_kind,
                parent,
                is_last_sibling,
                child_count: 0,
                marker: RowMarker::Truncated,
            });
            return;
        }
        self.rows.push(TreeRow {
            node_id: id.to_string(),
            depth,
            edge_kind,
            parent,
            is_last_sibling,
            child_count: kids.len(),
            marker: RowMarker::None,
        });
        self.path.push((id, row));
        let last = kids.len().saturating_sub(1);
        for (j, (child, kind)) in kids.into_iter().enumerate() {
            self.visit(child, depth + 1, Some(kind), Some(row), j == last);
        }
        self.path.pop();
    }
}

/// Indices of the rows to draw when the rows in `collapsed` hide their
/// descendants. Linear in the row count.
pub fn visible_rows(rows: &[TreeRow], collapsed: &HashSet<usize>) -> Vec<usize> {
    let mut hidden = vec![false; rows.len()];
    let mut out = Vec::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        let under_collapsed = row
            .parent
            .is_some_and(|p| hidden[p] || collapsed.contains(&p));
        hidden[i] = under_collapsed;
        if !under_collapsed {
            out.push(i);
        }
    }
    out
}

/// Rows that can be collapsed at all (they have children).
pub fn collapsible_rows(rows: &[TreeRow]) -> HashSet<usize> {
    rows.iter()
        .enumerate()
        .filter(|(_, row)| row.child_count > 0)
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_explorer::model::{
        DiffStatus, Edge, FlowMode, Node, NodeKind, Process, FLOW_SCHEMA_VERSION,
    };
    use crate::flow_explorer::test_support::load_fixture;

    fn flow(nodes: &[&str], edges: &[(&str, &str)], entries: &[&str]) -> Flow {
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

    fn ids(rows: &[TreeRow]) -> Vec<&str> {
        rows.iter().map(|r| r.node_id.as_str()).collect()
    }

    #[test]
    fn fixture_yields_one_row_per_node_in_call_order() {
        let flow = load_fixture();
        let rows = derive_tree(&flow);
        assert_eq!(rows.len(), 11);
        assert_eq!(
            ids(&rows),
            vec![
                "ui.cmd-enter",
                "Editor.tsx::handleKeyDown",
                "AgentPane.tsx::submit",
                "useAgentSession.ts::prompt",
                "rpc.sessions.prompt",
                "main.ts::RPCHandler.upgrade",
                "sessionsRouter.ts::prompt",
                "SessionRegistry.ts::open",
                "HaloAgentSession.ts::prompt",
                "rpc.sessions.prompt.resolves",
                "useAgentSession.ts::invalidateQueries",
            ]
        );
        assert_eq!(rows[0].depth, 0);
        assert_eq!(rows[0].edge_kind, None);
        assert_eq!(rows[1].edge_kind, Some(EdgeKind::HandledBy));
        assert_eq!(rows[1].depth, 1);
        // sessionsRouter.prompt has two children: open (not last) and prompt (last).
        assert_eq!(rows[6].child_count, 2);
        assert!(!rows[7].is_last_sibling);
        assert!(rows[8].is_last_sibling);
        assert_eq!(rows[7].parent, Some(6));
        assert_eq!(rows[8].parent, Some(6));
        assert_eq!(rows[9].edge_kind, Some(EdgeKind::Resolves));
        assert_eq!(rows[10].depth, 9);
        assert!(rows.iter().all(|r| r.marker == RowMarker::None));
    }

    #[test]
    fn diamond_emits_the_shared_node_twice_without_a_repeat_marker() {
        let f = flow(
            &["a", "b", "c", "d"],
            &[("a", "b"), ("a", "c"), ("b", "d"), ("c", "d")],
            &["a"],
        );
        let rows = derive_tree(&f);
        assert_eq!(ids(&rows), vec!["a", "b", "d", "c", "d"]);
        assert!(rows.iter().all(|r| r.marker == RowMarker::None));
        assert_eq!(rows[2].parent, Some(1));
        assert_eq!(rows[4].parent, Some(3));
        assert!(!rows[1].is_last_sibling);
        assert!(rows[3].is_last_sibling);
    }

    #[test]
    fn cycle_terminates_with_a_repeat_row() {
        let f = flow(&["a", "b"], &[("a", "b"), ("b", "a")], &["a"]);
        let rows = derive_tree(&f);
        assert_eq!(ids(&rows), vec!["a", "b", "a"]);
        assert_eq!(rows[2].marker, RowMarker::Repeat { row: 0 });
        assert_eq!(rows[2].child_count, 0);
        assert_eq!(rows[1].child_count, 1);
    }

    #[test]
    fn self_loop_is_a_repeat_of_itself() {
        let f = flow(&["a"], &[("a", "a")], &["a"]);
        let rows = derive_tree(&f);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].marker, RowMarker::Repeat { row: 0 });
    }

    #[test]
    fn multiple_entries_and_dangling_refs() {
        let f = flow(
            &["a", "b", "c"],
            &[("a", "ghost"), ("b", "c")],
            &["a", "missing", "b"],
        );
        let rows = derive_tree(&f);
        assert_eq!(ids(&rows), vec!["a", "b", "c"]);
        assert_eq!(rows[0].child_count, 0);
        assert!(!rows[0].is_last_sibling);
        assert!(rows[1].is_last_sibling);
        assert_eq!(rows[1].parent, None);
    }

    #[test]
    fn depth_cap_marks_the_deepest_row_truncated() {
        let names: Vec<String> = (0..MAX_TREE_DEPTH + 10).map(|i| format!("n{i}")).collect();
        let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let edges: Vec<(&str, &str)> = name_refs.windows(2).map(|w| (w[0], w[1])).collect();
        let f = flow(&name_refs, &edges, &[name_refs[0]]);
        let rows = derive_tree(&f);
        assert_eq!(rows.len(), MAX_TREE_DEPTH + 1);
        let last = rows.last().unwrap();
        assert_eq!(last.depth, MAX_TREE_DEPTH);
        assert_eq!(last.marker, RowMarker::Truncated);
        assert_eq!(last.child_count, 0);
        assert_eq!(rows[MAX_TREE_DEPTH - 1].marker, RowMarker::None);
    }

    #[test]
    fn a_leaf_at_the_depth_cap_is_not_truncated() {
        let names: Vec<String> = (0..=MAX_TREE_DEPTH).map(|i| format!("n{i}")).collect();
        let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let edges: Vec<(&str, &str)> = name_refs.windows(2).map(|w| (w[0], w[1])).collect();
        let f = flow(&name_refs, &edges, &[name_refs[0]]);
        let rows = derive_tree(&f);
        assert_eq!(rows.len(), MAX_TREE_DEPTH + 1);
        assert_eq!(rows.last().unwrap().marker, RowMarker::None);
    }

    #[test]
    fn row_cap_stops_the_walk_with_one_truncated_row() {
        let names: Vec<String> = (0..=MAX_TREE_ROWS + 5).map(|i| format!("n{i}")).collect();
        let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let edges: Vec<(&str, &str)> = name_refs[1..].iter().map(|c| (name_refs[0], *c)).collect();
        let f = flow(&name_refs, &edges, &[name_refs[0]]);
        let rows = derive_tree(&f);
        assert_eq!(rows.len(), MAX_TREE_ROWS + 1);
        assert_eq!(rows.last().unwrap().marker, RowMarker::Truncated);
        assert_eq!(rows.last().unwrap().depth, 1);
        assert_eq!(
            rows.iter()
                .filter(|r| r.marker == RowMarker::Truncated)
                .count(),
            1
        );
    }

    #[test]
    fn visible_rows_hides_descendants_of_collapsed_rows() {
        let flow = load_fixture();
        let rows = derive_tree(&flow);
        let all = visible_rows(&rows, &HashSet::new());
        assert_eq!(all.len(), rows.len());
        // Collapse sessionsRouter.prompt (row 6): rows 7..=10 disappear.
        let collapsed: HashSet<usize> = [6].into_iter().collect();
        assert_eq!(visible_rows(&rows, &collapsed), vec![0, 1, 2, 3, 4, 5, 6]);
        // Collapsing a hidden row changes nothing further.
        let collapsed: HashSet<usize> = [6, 8].into_iter().collect();
        assert_eq!(visible_rows(&rows, &collapsed), vec![0, 1, 2, 3, 4, 5, 6]);
        // Collapse all: only the entry remains.
        assert_eq!(visible_rows(&rows, &collapsible_rows(&rows)), vec![0]);
        // Every row but the two leaves has children.
        assert_eq!(collapsible_rows(&rows).len(), 9);
    }

    #[test]
    fn visible_rows_keeps_siblings_of_a_collapsed_row() {
        let f = flow(
            &["a", "b", "c", "d"],
            &[("a", "b"), ("b", "d"), ("a", "c")],
            &["a"],
        );
        let rows = derive_tree(&f);
        assert_eq!(ids(&rows), vec!["a", "b", "d", "c"]);
        let collapsed: HashSet<usize> = [1].into_iter().collect();
        assert_eq!(visible_rows(&rows, &collapsed), vec![0, 1, 3]);
    }
}
