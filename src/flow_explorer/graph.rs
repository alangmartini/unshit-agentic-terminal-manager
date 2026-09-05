//! Swim-lane layout for the Graph view.
//!
//! Pure geometry over the flow model: functions (and state) become boxes
//! in their process lane, events fold into the edge between their emitter
//! and their handler (numbered in discovery order, like the reference UI),
//! and everything is measured in CSS pixels so the renderer only positions
//! absolute children. Deterministic for a given (flow, roots, depth).

use std::collections::{HashMap, VecDeque};

use super::model::{EdgeKind, Flow, NodeKind};

pub const LANE_WIDTH: f32 = 260.0;
pub const LANE_GAP: f32 = 32.0;
/// Room for the lane pill above the first row.
pub const LANE_HEADER: f32 = 48.0;
pub const NODE_WIDTH: f32 = 220.0;
pub const NODE_HEIGHT: f32 = 56.0;
/// Gap between boxes stacked in one (lane, row) cell.
pub const NODE_STACK_GAP: f32 = 10.0;
pub const ROW_GAP: f32 = 44.0;
pub const PADDING: f32 = 16.0;
/// Dash and gap length of a dotted edge, in px along the curve.
pub const DASH: f32 = 6.0;
const FLATTEN_STEPS: usize = 32;

/// `Depth 1 2 3 all` filter: how many hops from the root stay visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DepthFilter {
    One,
    Two,
    Three,
    #[default]
    All,
}

impl DepthFilter {
    pub const ALL: [DepthFilter; 4] = [
        DepthFilter::One,
        DepthFilter::Two,
        DepthFilter::Three,
        DepthFilter::All,
    ];

    /// Name used in `flow.graph.depth:<name>`.
    pub fn as_str(self) -> &'static str {
        match self {
            DepthFilter::One => "1",
            DepthFilter::Two => "2",
            DepthFilter::Three => "3",
            DepthFilter::All => "all",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.as_str() == name)
    }

    pub fn limit(self) -> Option<usize> {
        match self {
            DepthFilter::One => Some(1),
            DepthFilter::Two => Some(2),
            DepthFilter::Three => Some(3),
            DepthFilter::All => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn top_center(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y)
    }

    pub fn bottom_center(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h)
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.x + other.w
            && other.x < self.x + self.w
            && self.y < other.y + other.h
            && other.y < self.y + self.h
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LaneGeom {
    pub process_id: String,
    pub label: String,
    /// Index into the flow's process list (drives the colour class); the
    /// synthetic lane for undeclared processes uses `processes.len()`.
    pub palette: usize,
    pub rect: Rect,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeGeom {
    pub node_id: String,
    pub depth: usize,
    pub lane: usize,
    pub rect: Rect,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EdgeGeom {
    pub from: String,
    pub to: String,
    /// The event folded into this edge, when it carries one.
    pub event_id: Option<String>,
    /// Sequence badge, event edges only.
    pub number: Option<u32>,
    /// `RPC · sessions.prompt`, event edges only.
    pub label: Option<String>,
    /// `resolves` edges are dotted.
    pub dotted: bool,
    /// SVG path data in canvas pixels.
    pub path: String,
    /// Where the badge (or label) sits: the curve's midpoint.
    pub badge: (f32, f32),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct GraphLayout {
    pub lanes: Vec<LaneGeom>,
    pub nodes: Vec<NodeGeom>,
    pub edges: Vec<EdgeGeom>,
    pub width: f32,
    pub height: f32,
}

impl GraphLayout {
    pub fn node(&self, id: &str) -> Option<&NodeGeom> {
        self.nodes.iter().find(|n| n.node_id == id)
    }
}

struct Link {
    from: String,
    to: String,
    event_id: Option<String>,
    dotted: bool,
}

/// Lay out the subgraph reachable from `roots` within `depth` hops.
///
/// A root that is an event (the usual entry) is drawn as a box so the
/// first edge has somewhere to start; every other event folds into the
/// edge from its emitter to each handler. Events without a handler stay
/// boxes so nothing silently disappears.
pub fn layout(flow: &Flow, roots: &[String], depth: DepthFilter) -> GraphLayout {
    let limit = depth.limit();
    let mut depth_of: HashMap<&str, usize> = HashMap::new();
    let mut order: Vec<&str> = Vec::new();
    let mut queue: VecDeque<&str> = VecDeque::new();
    let mut links: Vec<Link> = Vec::new();

    for root in roots {
        if let Some(node) = flow.node(root) {
            if !depth_of.contains_key(node.id.as_str()) {
                depth_of.insert(&node.id, 0);
                order.push(&node.id);
                queue.push_back(&node.id);
            }
        }
    }

    while let Some(id) = queue.pop_front() {
        let d = depth_of[id];
        let next = d + 1;
        if limit.is_some_and(|max| next > max) {
            continue;
        }
        let Some(source) = flow.node(id) else {
            continue;
        };
        for (_, edge) in flow.outgoing(id) {
            let Some(target) = flow.node(&edge.to) else {
                continue;
            };
            let handlers: Vec<&str> = if target.kind == NodeKind::Event
                && source.kind != NodeKind::Event
            {
                flow.outgoing(&target.id)
                    .into_iter()
                    .filter(|(_, e)| e.kind == EdgeKind::HandledBy && flow.node(&e.to).is_some())
                    .map(|(_, e)| e.to.as_str())
                    .collect()
            } else {
                Vec::new()
            };
            let dotted = edge.kind == EdgeKind::Resolves;
            if handlers.is_empty() {
                // A plain call, a dangling event, or an event root's own
                // handler edge (which carries the root as its event).
                let event_id = (source.kind == NodeKind::Event).then(|| source.id.clone());
                links.push(Link {
                    from: source.id.clone(),
                    to: target.id.clone(),
                    event_id,
                    dotted,
                });
                visit(
                    flow,
                    &target.id,
                    next,
                    &mut depth_of,
                    &mut order,
                    &mut queue,
                );
            } else {
                for handler in handlers {
                    links.push(Link {
                        from: source.id.clone(),
                        to: handler.to_string(),
                        event_id: Some(target.id.clone()),
                        dotted,
                    });
                    visit(flow, handler, next, &mut depth_of, &mut order, &mut queue);
                }
            }
        }
    }

    // Lanes: declared processes that hold a box, in declaration order,
    // then one synthetic lane for boxes without a declared process.
    let lane_of_node = |id: &str| -> String {
        flow.node(id)
            .and_then(|n| n.process.clone())
            .filter(|p| flow.process(p).is_some())
            .unwrap_or_else(|| "outside".to_string())
    };
    let mut lanes: Vec<LaneGeom> = Vec::new();
    for (index, process) in flow.processes.iter().enumerate() {
        if order.iter().any(|id| lane_of_node(id) == process.id) {
            lanes.push(LaneGeom {
                process_id: process.id.clone(),
                label: process.label.clone(),
                palette: index,
                rect: Rect::default(),
            });
        }
    }
    if order
        .iter()
        .any(|id| flow.process(&lane_of_node(id)).is_none())
    {
        lanes.push(LaneGeom {
            process_id: "outside".to_string(),
            label: "outside".to_string(),
            palette: flow.processes.len(),
            rect: Rect::default(),
        });
    }
    let lane_index = |id: &str| -> usize {
        let process = lane_of_node(id);
        lanes
            .iter()
            .position(|l| l.process_id == process)
            .unwrap_or(0)
    };

    // Rows: one per depth; boxes sharing a (lane, depth) cell stack, and
    // the row grows to the tallest stack.
    let max_depth = order.iter().map(|id| depth_of[id]).max().unwrap_or(0);
    let mut stack_counts: HashMap<(usize, usize), usize> = HashMap::new();
    let mut placements: Vec<(&str, usize, usize, usize)> = Vec::new(); // id, lane, depth, stack
    for id in &order {
        let lane = lane_index(id);
        let d = depth_of[id];
        let slot = stack_counts.entry((lane, d)).or_insert(0);
        placements.push((id, lane, d, *slot));
        *slot += 1;
    }
    let mut row_heights = vec![0.0f32; max_depth + 1];
    for ((_, d), count) in &stack_counts {
        let height = *count as f32 * NODE_HEIGHT + count.saturating_sub(1) as f32 * NODE_STACK_GAP;
        row_heights[*d] = row_heights[*d].max(height);
    }
    let mut row_y = Vec::with_capacity(row_heights.len());
    let mut y = PADDING + LANE_HEADER;
    for height in &row_heights {
        row_y.push(y);
        y += height + ROW_GAP;
    }
    let height = if order.is_empty() {
        PADDING * 2.0 + LANE_HEADER
    } else {
        y - ROW_GAP + PADDING
    };
    let lane_count = lanes.len().max(1);
    let width = PADDING * 2.0 + lane_count as f32 * LANE_WIDTH + (lane_count - 1) as f32 * LANE_GAP;
    for (i, lane) in lanes.iter_mut().enumerate() {
        lane.rect = Rect {
            x: PADDING + i as f32 * (LANE_WIDTH + LANE_GAP),
            y: PADDING,
            w: LANE_WIDTH,
            h: height - PADDING * 2.0,
        };
    }

    let nodes: Vec<NodeGeom> = placements
        .iter()
        .map(|(id, lane, d, stack)| NodeGeom {
            node_id: id.to_string(),
            depth: *d,
            lane: *lane,
            rect: Rect {
                x: PADDING
                    + *lane as f32 * (LANE_WIDTH + LANE_GAP)
                    + (LANE_WIDTH - NODE_WIDTH) / 2.0,
                y: row_y[*d] + *stack as f32 * (NODE_HEIGHT + NODE_STACK_GAP),
                w: NODE_WIDTH,
                h: NODE_HEIGHT,
            },
        })
        .collect();
    let rect_of = |id: &str| nodes.iter().find(|n| n.node_id == id).map(|n| n.rect);

    let mut number = 0u32;
    let mut edges = Vec::with_capacity(links.len());
    for link in links {
        let (Some(from), Some(to)) = (rect_of(&link.from), rect_of(&link.to)) else {
            continue;
        };
        let (sx, sy) = from.bottom_center();
        let (tx, ty) = to.top_center();
        let reach = (ty - sy).abs().max(48.0) * 0.4;
        let p0 = (sx, sy);
        let p1 = (sx, sy + reach);
        let p2 = (tx, ty - reach);
        let p3 = (tx, ty);
        let path = if link.dotted {
            dotted_path(&flatten_cubic(p0, p1, p2, p3))
        } else {
            format!(
                "M {} {} C {} {} {} {} {} {}",
                px(p0.0),
                px(p0.1),
                px(p1.0),
                px(p1.1),
                px(p2.0),
                px(p2.1),
                px(p3.0),
                px(p3.1)
            )
        };
        let (number_of, label) = match link.event_id.as_deref().and_then(|id| flow.node(id)) {
            Some(event) => {
                number += 1;
                let carrier = event
                    .carrier
                    .map(|c| c.label())
                    .unwrap_or(event.kind.as_str());
                (
                    Some(number),
                    Some(format!("{carrier} \u{00B7} {}", event.name)),
                )
            }
            None => (None, None),
        };
        edges.push(EdgeGeom {
            from: link.from,
            to: link.to,
            event_id: link.event_id,
            number: number_of,
            label,
            dotted: link.dotted,
            path,
            badge: cubic_point(p0, p1, p2, p3, 0.5),
        });
    }

    GraphLayout {
        lanes,
        nodes,
        edges,
        width,
        height,
    }
}

#[allow(clippy::too_many_arguments)]
fn visit<'a>(
    flow: &'a Flow,
    id: &str,
    depth: usize,
    depth_of: &mut HashMap<&'a str, usize>,
    order: &mut Vec<&'a str>,
    queue: &mut VecDeque<&'a str>,
) {
    let Some(node) = flow.node(id) else {
        return;
    };
    if depth_of.contains_key(node.id.as_str()) {
        return;
    }
    depth_of.insert(&node.id, depth);
    order.push(&node.id);
    queue.push_back(&node.id);
}

fn px(v: f32) -> String {
    format!("{v:.1}")
}

/// Point on a cubic Bézier at `t` (De Casteljau).
pub fn cubic_point(
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
    t: f32,
) -> (f32, f32) {
    let u = 1.0 - t;
    let a = u * u * u;
    let b = 3.0 * u * u * t;
    let c = 3.0 * u * t * t;
    let d = t * t * t;
    (
        a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0,
        a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1,
    )
}

fn flatten_cubic(
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
) -> Vec<(f32, f32)> {
    (0..=FLATTEN_STEPS)
        .map(|i| cubic_point(p0, p1, p2, p3, i as f32 / FLATTEN_STEPS as f32))
        .collect()
}

/// `stroke-dasharray` is unsupported by the SVG engine, so a dotted edge
/// is many short `M … L …` sub-paths sampled along the flattened curve:
/// `DASH` px on, `DASH` px off.
pub fn dotted_path(points: &[(f32, f32)]) -> String {
    let mut out = String::new();
    let mut on = true;
    let mut remaining = DASH;
    let mut pen: Option<(f32, f32)> = None;
    for pair in points.windows(2) {
        let (mut x, mut y) = pair[0];
        let (ex, ey) = pair[1];
        let mut seg_len = ((ex - x).powi(2) + (ey - y).powi(2)).sqrt();
        while seg_len > 0.0 {
            let step = seg_len.min(remaining);
            let t = step / seg_len;
            let nx = x + (ex - x) * t;
            let ny = y + (ey - y) * t;
            if on {
                match pen {
                    Some(_) => {}
                    None => {
                        out.push_str(&format!("M {} {} ", px(x), px(y)));
                    }
                }
                out.push_str(&format!("L {} {} ", px(nx), px(ny)));
                pen = Some((nx, ny));
            }
            x = nx;
            y = ny;
            seg_len -= step;
            remaining -= step;
            if remaining <= 0.0 {
                on = !on;
                remaining = DASH;
                pen = None;
            }
        }
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_explorer::model::{Edge, Node, Process};
    use crate::flow_explorer::test_support::load_fixture;

    fn ids(nodes: &[NodeGeom]) -> Vec<&str> {
        nodes.iter().map(|n| n.node_id.as_str()).collect()
    }

    #[test]
    fn fixture_folds_events_into_numbered_edges() {
        let flow = load_fixture();
        let g = layout(&flow, &flow.entries, DepthFilter::All);
        assert_eq!(
            ids(&g.nodes),
            vec![
                "ui.cmd-enter",
                "Editor.tsx::handleKeyDown",
                "AgentPane.tsx::submit",
                "useAgentSession.ts::prompt",
                "main.ts::RPCHandler.upgrade",
                "sessionsRouter.ts::prompt",
                "SessionRegistry.ts::open",
                "HaloAgentSession.ts::prompt",
                "useAgentSession.ts::invalidateQueries",
            ],
            "functions and the entry event are boxes; other events fold"
        );
        let lanes: Vec<&str> = g.lanes.iter().map(|l| l.process_id.as_str()).collect();
        assert_eq!(
            lanes,
            vec!["outside", "renderer", "main"],
            "preload is empty"
        );
        assert_eq!(g.edges.len(), 8);
        let numbered: Vec<(u32, &str, bool)> = g
            .edges
            .iter()
            .filter_map(|e| Some((e.number?, e.label.as_deref()?, e.dotted)))
            .collect();
        assert_eq!(
            numbered,
            vec![
                (1, "UI \u{00B7} Cmd/Ctrl+Enter in the composer", false),
                (2, "RPC \u{00B7} sessions.prompt", false),
                (3, "RPC \u{00B7} sessions.prompt resolves", true),
            ]
        );
        let resolves = g.edges.iter().find(|e| e.dotted).unwrap();
        assert_eq!(resolves.from, "HaloAgentSession.ts::prompt");
        assert_eq!(resolves.to, "useAgentSession.ts::invalidateQueries");
        assert!(resolves.path.starts_with("M ") && resolves.path.matches("M ").count() > 3);
        let plain = g.edges.iter().find(|e| e.number.is_none()).unwrap();
        assert!(plain.path.starts_with("M ") && plain.path.contains(" C "));
        assert!(g.width > 3.0 * LANE_WIDTH && g.height > 5.0 * NODE_HEIGHT);
    }

    #[test]
    fn lanes_rows_and_stacks_are_deterministic_and_disjoint() {
        let flow = load_fixture();
        let a = layout(&flow, &flow.entries, DepthFilter::All);
        let b = layout(&flow, &flow.entries, DepthFilter::All);
        assert_eq!(a, b);
        for (i, n) in a.nodes.iter().enumerate() {
            for m in &a.nodes[i + 1..] {
                assert!(
                    !n.rect.intersects(&m.rect),
                    "{} overlaps {}",
                    n.node_id,
                    m.node_id
                );
            }
            let lane = &a.lanes[n.lane].rect;
            assert!(n.rect.x >= lane.x && n.rect.x + n.rect.w <= lane.x + lane.w);
        }
        // sessionsRouter calls two functions in the main lane: same row,
        // stacked.
        let open = a.node("SessionRegistry.ts::open").unwrap();
        let prompt = a.node("HaloAgentSession.ts::prompt").unwrap();
        assert_eq!(open.depth, prompt.depth);
        assert_eq!(open.rect.x, prompt.rect.x);
        assert!(prompt.rect.y > open.rect.y + open.rect.h);
        // The rows below the stack start under it.
        let after = a.node("useAgentSession.ts::invalidateQueries").unwrap();
        assert!(after.rect.y > prompt.rect.y + prompt.rect.h);
    }

    #[test]
    fn depth_filter_bounds_the_subgraph() {
        let flow = load_fixture();
        let one = layout(&flow, &flow.entries, DepthFilter::One);
        assert_eq!(
            ids(&one.nodes),
            vec!["ui.cmd-enter", "Editor.tsx::handleKeyDown"]
        );
        assert_eq!(one.edges.len(), 1);
        assert_eq!(one.edges[0].number, Some(1));
        let three = layout(&flow, &flow.entries, DepthFilter::Three);
        assert_eq!(three.nodes.len(), 4);
        assert_eq!(three.edges.len(), 3);
    }

    #[test]
    fn zooming_into_a_receiver_relays_out_its_subgraph() {
        let flow = load_fixture();
        let g = layout(
            &flow,
            &["main.ts::RPCHandler.upgrade".to_string()],
            DepthFilter::All,
        );
        assert_eq!(
            ids(&g.nodes),
            vec![
                "main.ts::RPCHandler.upgrade",
                "sessionsRouter.ts::prompt",
                "SessionRegistry.ts::open",
                "HaloAgentSession.ts::prompt",
                "useAgentSession.ts::invalidateQueries",
            ]
        );
        let lanes: Vec<&str> = g.lanes.iter().map(|l| l.process_id.as_str()).collect();
        assert_eq!(
            lanes,
            vec!["renderer", "main"],
            "declaration order, not discovery order"
        );
        assert_eq!(g.node("main.ts::RPCHandler.upgrade").unwrap().depth, 0);
        assert_eq!(g.edges.iter().filter(|e| e.number.is_some()).count(), 1);
        assert!(layout(&flow, &["nope".to_string()], DepthFilter::All)
            .nodes
            .is_empty());
    }

    #[test]
    fn cycles_terminate_and_undeclared_processes_get_a_lane() {
        let mut flow = load_fixture();
        flow.processes = vec![Process {
            id: "renderer".into(),
            label: "Renderer".into(),
        }];
        flow.nodes = vec![
            Node {
                id: "a".into(),
                name: "a".into(),
                kind: NodeKind::Function,
                process: Some("renderer".into()),
                carrier: None,
                description: String::new(),
                tags: vec![],
                location: None,
                status: Default::default(),
                hidden_children: 0,
                payload: None,
            },
            Node {
                id: "b".into(),
                name: "b".into(),
                kind: NodeKind::Function,
                process: None,
                carrier: None,
                description: String::new(),
                tags: vec![],
                location: None,
                status: Default::default(),
                hidden_children: 0,
                payload: None,
            },
        ];
        flow.edges = vec![
            Edge {
                from: "a".into(),
                to: "b".into(),
                kind: EdgeKind::Calls,
                label: None,
            },
            Edge {
                from: "b".into(),
                to: "a".into(),
                kind: EdgeKind::Calls,
                label: None,
            },
        ];
        flow.entries = vec!["a".into()];
        let g = layout(&flow, &flow.entries, DepthFilter::All);
        assert_eq!(ids(&g.nodes), vec!["a", "b"]);
        assert_eq!(g.edges.len(), 2, "the back edge is drawn once");
        let lanes: Vec<&str> = g.lanes.iter().map(|l| l.label.as_str()).collect();
        assert_eq!(lanes, vec!["Renderer", "outside"]);
        assert_eq!(g.lanes[1].palette, 1);
        let back = &g.edges[1];
        assert_eq!((back.from.as_str(), back.to.as_str()), ("b", "a"));
        assert!(back.path.contains(" C "));
    }

    #[test]
    fn cubic_midpoint_and_dash_segmentation() {
        let p0 = (0.0, 0.0);
        let p1 = (0.0, 40.0);
        let p2 = (100.0, 60.0);
        let p3 = (100.0, 100.0);
        let (x, y) = cubic_point(p0, p1, p2, p3, 0.5);
        assert!((x - 50.0).abs() < 1e-4, "{x}");
        assert!(
            (y - (0.0 + 3.0 * 40.0 + 3.0 * 60.0 + 100.0) / 8.0).abs() < 1e-4,
            "{y}"
        );
        assert_eq!(cubic_point(p0, p1, p2, p3, 0.0), p0);
        assert_eq!(cubic_point(p0, p1, p2, p3, 1.0), p3);

        let line = [(0.0, 0.0), (60.0, 0.0)];
        let d = dotted_path(&line);
        assert_eq!(d.matches("M ").count(), 5, "{d}");
        assert!(d.starts_with("M 0.0 0.0 L 6.0 0.0"), "{d}");
        assert!(d.ends_with("L 54.0 0.0"), "{d}");
        assert_eq!(dotted_path(&[(1.0, 1.0)]), "");
    }
}
