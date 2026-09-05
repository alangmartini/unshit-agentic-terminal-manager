//! Flow Explorer pane body: header, view/level toolbar, the call-stack
//! tree and the legend. Every interaction dispatches a `flow.*` command,
//! so clicks, keys, tests and `TM_STARTUP_DISPATCH` share one path.

use unshit::core::element::*;
use unshit::core::event::{Event, EventType, Key, KeyEventKind, KeyboardEvent, Modifiers};
use unshit::core::style::parse::StyleDeclaration;
use unshit::core::style::types::Dimension;
use unshit::core::svg::{
    parse_svg_path, StrokeLineCap, StrokeLineJoin, SvgAttrs, SvgNode, SvgPaint, SvgPrimitive,
    ViewBox,
};

use crate::flow_explorer::{
    tokenize, Carrier, ColumnItem, ColumnSection, DepthFilter, DiffStatus, DisplayRow, FlowLevel,
    FlowMode, FlowPane, FlowView, GraphLayout, Node, NodeKind, Rect, RowMarker, Snippet,
    SnippetError,
};
use crate::state::{mutate_with, PaneId, SharedState};
use crate::ui::icons::{icon_carrier, svg_icon};

/// Process colours cycle through this many `.flow-proc-<n>` classes.
const PROCESS_PALETTE: usize = 6;

/// The six view tabs in the order the reference UI shows them; `None`
/// renders a disabled tab for a view this build does not implement.
const VIEW_TABS: [(&str, Option<FlowView>); 6] = [
    ("stack graph", None),
    ("tree", None),
    ("call stack", Some(FlowView::CallStack)),
    ("panes", Some(FlowView::Panes)),
    ("graph", Some(FlowView::Graph)),
    ("sequence", None),
];

const LEVEL_TABS: [FlowLevel; 3] = [FlowLevel::Events, FlowLevel::Code, FlowLevel::Source];

/// `capture_keyboard` is true for the active pane: its body then owns
/// the keys listed in [`flow_key_command`] (global keybinds still win).
pub fn build_flow_pane_body(
    pane_id: PaneId,
    capture_keyboard: bool,
    pane: &FlowPane,
    shared: &SharedState,
) -> ElementDef {
    let mut body = ElementDef::new(Tag::Div)
        .with_class("pane-body")
        .with_class("flow-pane")
        .with_id(format!("flow-pane-{}", pane_id.0))
        // Focusability requires a tab index (see terminal_grid.rs).
        .with_tab_index(0);
    if capture_keyboard {
        body = body.captures_keyboard(true);
        let kbd_shared = shared.clone();
        body = body.on(
            EventType::KeyboardCapture,
            move |event: &Event| -> Option<Box<dyn std::any::Any>> {
                let Event::Keyboard(kb) = event else {
                    return None;
                };
                if kb.kind != KeyEventKind::Pressed {
                    return None;
                }
                let changed = mutate_with(&kbd_shared, |st| handle_flow_key(st, pane_id.0, kb));
                match changed {
                    Some(true) => Some(Box::new(unshit::core::event::RequestRebuild)),
                    _ => None,
                }
            },
        );
    }
    let body = body
        .with_child(build_header(pane))
        .with_child(build_toolbar(pane, shared));
    let body = match pane.view {
        FlowView::CallStack => body.with_child(build_call_stack(pane, shared)),
        FlowView::Panes => body.with_child(build_panes(pane, shared)),
        FlowView::Graph => body.with_child(build_graph(pane, shared)),
    };
    body.with_child(build_legend(pane))
}

/// Keys the active Flow Explorer pane handles while it holds keyboard
/// capture. Returns `None` for keys it does not own (the framework then
/// falls through to global keybinds), `Some(changed)` otherwise. Every
/// key maps onto a `flow.*` dispatch so tests and `TM_STARTUP_DISPATCH`
/// exercise the same transitions.
pub(crate) fn handle_flow_key(
    st: &mut crate::state::AppState,
    pane_id: u32,
    kb: &KeyboardEvent,
) -> Option<bool> {
    if !st.flows.contains_key(&pane_id) || st.active_pane.0 != pane_id {
        return None;
    }
    let command = flow_key_command(kb)?;
    Some(crate::state::dispatch(st, command))
}

/// Pure key → command mapping (plain digits stay free for the global
/// keybinds; views use Ctrl+1/2/3).
pub(crate) fn flow_key_command(kb: &KeyboardEvent) -> Option<&'static str> {
    let ctrl = kb.modifiers.contains(Modifiers::CTRL);
    let alt = kb.modifiers.contains(Modifiers::ALT);
    let shift = kb.modifiers.contains(Modifiers::SHIFT);
    if alt {
        return None;
    }
    let plain = !ctrl && !shift;
    Some(match kb.key {
        Key::ArrowDown if plain => "flow.select_move:1",
        Key::ArrowUp if plain => "flow.select_move:-1",
        Key::PageDown if plain => "flow.select_move:10",
        Key::PageUp if plain => "flow.select_move:-10",
        Key::Home if plain => "flow.select_first",
        Key::End if plain => "flow.select_last",
        Key::ArrowRight if plain => "flow.select_into",
        Key::ArrowLeft if plain => "flow.select_out",
        Key::Enter | Key::Space if plain => "flow.toggle_selected",
        Key::Escape if plain => "flow.select_none",
        Key::Char('s') | Key::Char('S') if plain => "flow.src_selected",
        Key::Char('e') | Key::Char('E') if plain => "flow.expand_all",
        Key::Char('c') | Key::Char('C') if plain => "flow.collapse_all",
        Key::Char('1') if ctrl && !shift => "flow.view:stack",
        Key::Char('2') if ctrl && !shift => "flow.view:panes",
        Key::Char('3') if ctrl && !shift => "flow.view:graph",
        _ => return None,
    })
}

fn build_header(pane: &FlowPane) -> ElementDef {
    let flow = &pane.flow;
    let mut header = ElementDef::new(Tag::Div)
        .with_class("flow-header")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("flow-title")
                .with_text(flow.title.clone()),
        );
    if !flow.summary.trim().is_empty() {
        header = header.with_child(
            ElementDef::new(Tag::Span)
                .with_class("flow-summary")
                .with_text(flow.summary.clone()),
        );
    }
    header.with_child(
        ElementDef::new(Tag::Span)
            .with_class("flow-meta")
            .with_text(meta_line(pane)),
    )
}

/// `11 nodes · 10 edges · explain · main@a1b2c3d`
fn meta_line(pane: &FlowPane) -> String {
    let flow = &pane.flow;
    let mut parts = vec![
        format!("{} nodes", flow.nodes.len()),
        format!("{} edges", flow.edges.len()),
        flow.mode.as_str().to_string(),
    ];
    if let Some(git_ref) = &flow.git_ref {
        parts.push(git_ref.clone());
    }
    if let Some(range) = &flow.diff_range {
        parts.push(format!("{}..{}", range.base, range.head));
    }
    parts.join(" \u{00B7} ")
}

/// Two rows: the view tabs, then the level tabs with expand/collapse on
/// the right. One row does not fit a half-width pane.
fn build_toolbar(pane: &FlowPane, shared: &SharedState) -> ElementDef {
    let mut views = ElementDef::new(Tag::Div)
        .with_class("flow-toolbar-row")
        .with_child(toolbar_label("view:"));
    for (name, view) in VIEW_TABS {
        views = views.with_child(match view {
            Some(view) => tab_button(
                name,
                view == pane.view,
                shared,
                format!("flow.view:{}", view.as_str()),
            ),
            None => ElementDef::new(Tag::Button)
                .with_class("flow-tab")
                .with_class("disabled")
                .with_text(name),
        });
    }
    let mut levels = ElementDef::new(Tag::Div)
        .with_class("flow-toolbar-row")
        .with_child(toolbar_label("level:"));
    for level in LEVEL_TABS {
        levels = levels.with_child(tab_button(
            level.as_str(),
            level == pane.level,
            shared,
            format!("flow.level:{}", level.as_str()),
        ));
    }
    levels = levels
        .with_child(ElementDef::new(Tag::Div).with_class("flow-toolbar-spacer"))
        .with_child(tool_button("expand all", shared, "flow.expand_all"))
        .with_child(tool_button("collapse all", shared, "flow.collapse_all"));
    ElementDef::new(Tag::Div)
        .with_class("flow-toolbar")
        .with_child(views)
        .with_child(levels)
}

fn toolbar_label(text: &str) -> ElementDef {
    ElementDef::new(Tag::Span)
        .with_class("flow-toolbar-label")
        .with_text(text)
}

fn tab_button(name: &str, active: bool, shared: &SharedState, command: String) -> ElementDef {
    let mut button = ElementDef::new(Tag::Button)
        .with_class("flow-tab")
        .with_text(name);
    if active {
        button = button.with_class("active");
    }
    dispatch_on_click(button, shared, command)
}

fn tool_button(name: &str, shared: &SharedState, command: &str) -> ElementDef {
    let button = ElementDef::new(Tag::Button)
        .with_class("flow-tool")
        .with_text(name);
    dispatch_on_click(button, shared, command.to_string())
}

fn dispatch_on_click(el: ElementDef, shared: &SharedState, command: String) -> ElementDef {
    let shared = shared.clone();
    el.on_click(move || {
        mutate_with(&shared, |st| {
            crate::state::dispatch(st, &command);
        });
    })
}

/// Swim-lane graph: breadcrumb and depth filter on top, then a scrolling
/// canvas of absolutely positioned lanes, boxes, badges and one SVG layer
/// for the edges. Geometry comes from [`FlowPane::graph_layout`].
fn build_graph(pane: &FlowPane, shared: &SharedState) -> ElementDef {
    let graph = pane.graph_layout();
    let root = ElementDef::new(Tag::Div)
        .with_class("flow-graph")
        .with_child(build_graph_bar(pane, shared));
    if graph.nodes.is_empty() {
        return root.with_child(
            ElementDef::new(Tag::Div)
                .with_class("flow-tree")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("flow-placeholder")
                        .with_text("nothing reachable from this root"),
                ),
        );
    }
    let mut canvas = ElementDef::new(Tag::Div)
        .with_class("flow-graph-canvas")
        .with_style(StyleDeclaration::Width(Dimension::Px(graph.width)))
        .with_style(StyleDeclaration::Height(Dimension::Px(graph.height)));
    for lane in &graph.lanes {
        canvas = canvas.with_child(
            place(ElementDef::new(Tag::Div), &lane.rect)
                .with_class("flow-lane")
                .with_class(process_class(lane.palette))
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("flow-lane-pill")
                        .with_text(lane.label.clone()),
                ),
        );
    }
    canvas = canvas.with_child(build_graph_edges(&graph));
    let review = pane.flow.mode == FlowMode::Review;
    for node_geom in &graph.nodes {
        let Some(node) = pane.flow.node(&node_geom.node_id) else {
            continue;
        };
        let mut el =
            place(ElementDef::new(Tag::Div), &node_geom.rect).with_class("flow-graph-node");
        if let Some(class) = node_process_class(pane, node) {
            el = el.with_class(class);
        }
        if node.kind == NodeKind::Event {
            el = el.with_class("event");
        }
        if review && node.status != DiffStatus::Same {
            el = el.with_class(format!("diff-{}", node.status.slug()));
        }
        el = el.with_child(
            ElementDef::new(Tag::Span)
                .with_class("flow-graph-node-name")
                .with_text(node.name.clone()),
        );
        if !node.description.trim().is_empty() {
            el = el.with_child(
                ElementDef::new(Tag::Span)
                    .with_class("flow-graph-node-desc")
                    .with_text(node.description.clone()),
            );
        }
        let command = format!("flow.graph.details:{}", node.id);
        canvas = canvas.with_child(dispatch_on_click(el, shared, command));
    }
    for edge in &graph.edges {
        let (Some(number), Some(label)) = (edge.number, edge.label.as_deref()) else {
            continue;
        };
        let (bx, by) = edge.badge;
        let badge = Rect {
            x: bx - GRAPH_BADGE / 2.0,
            y: by - GRAPH_BADGE / 2.0,
            w: GRAPH_BADGE,
            h: GRAPH_BADGE,
        };
        let badge_el = place(ElementDef::new(Tag::Button), &badge)
            .with_class("flow-graph-badge")
            .with_text(number.to_string());
        let command = format!("flow.graph.zoom:{}", edge.to);
        canvas = canvas.with_child(dispatch_on_click(badge_el, shared, command));
        let label_rect = Rect {
            x: bx + GRAPH_BADGE / 2.0 + 6.0,
            y: by - 8.0,
            w: GRAPH_LABEL_WIDTH,
            h: 16.0,
        };
        canvas = canvas.with_child(
            place(ElementDef::new(Tag::Span), &label_rect)
                .with_class("flow-graph-edge-label")
                .with_text(label.to_string()),
        );
    }
    root.with_child(
        ElementDef::new(Tag::Div)
            .with_class("flow-graph-scroll")
            .with_child(canvas),
    )
}

/// Badge diameter in px.
const GRAPH_BADGE: f32 = 18.0;
/// Width reserved for an edge label next to its badge.
const GRAPH_LABEL_WIDTH: f32 = 200.0;

fn place(el: ElementDef, rect: &Rect) -> ElementDef {
    el.with_style(StyleDeclaration::Left(Dimension::Px(rect.x)))
        .with_style(StyleDeclaration::Top(Dimension::Px(rect.y)))
        .with_style(StyleDeclaration::Width(Dimension::Px(rect.w)))
        .with_style(StyleDeclaration::Height(Dimension::Px(rect.h)))
}

/// Breadcrumb (flow title, then each zoomed receiver), the hint, and the
/// depth filter.
fn build_graph_bar(pane: &FlowPane, shared: &SharedState) -> ElementDef {
    let mut crumbs = ElementDef::new(Tag::Div).with_class("flow-crumbs");
    let mut names = vec![pane.flow.title.clone()];
    for id in &pane.graph_crumbs {
        names.push(
            pane.flow
                .node(id)
                .map(|n| n.name.clone())
                .unwrap_or_else(|| id.clone()),
        );
    }
    let last = names.len() - 1;
    for (index, name) in names.into_iter().enumerate() {
        if index > 0 {
            crumbs = crumbs.with_child(
                ElementDef::new(Tag::Span)
                    .with_class("flow-crumb-sep")
                    .with_text("\u{203A}"),
            );
        }
        let mut crumb = ElementDef::new(Tag::Button)
            .with_class("flow-crumb")
            .with_text(name);
        if index == last {
            crumb = crumb.with_class("current");
        } else {
            crumb = dispatch_on_click(crumb, shared, format!("flow.graph.crumb:{index}"));
        }
        crumbs = crumbs.with_child(crumb);
    }
    let tools = ElementDef::new(Tag::Div)
        .with_class("flow-graph-tools")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("flow-graph-hint")
                .with_text(
                    "click a numbered event to zoom into what its receiver does. \
                     click a box for details.",
                ),
        );
    let mut depth_group = ElementDef::new(Tag::Div)
        .with_class("flow-depth-group")
        .with_child(toolbar_label("depth:"));
    for depth in DepthFilter::ALL {
        depth_group = depth_group.with_child(
            tab_button(
                depth.as_str(),
                pane.graph_depth == depth,
                shared,
                format!("flow.graph.depth:{}", depth.as_str()),
            )
            .with_class("flow-depth"),
        );
    }
    let tools = tools.with_child(depth_group);
    ElementDef::new(Tag::Div)
        .with_class("flow-graph-bar")
        .with_child(crumbs)
        .with_child(tools)
}

/// One SVG layer with every edge path; the viewBox matches the canvas so
/// path coordinates are canvas pixels.
fn build_graph_edges(graph: &GraphLayout) -> ElementDef {
    let paths = graph
        .edges
        .iter()
        .map(|edge| SvgNode {
            primitive: SvgPrimitive::Path {
                d: edge.path.clone(),
                commands: parse_svg_path(&edge.path).expect("layout emits valid path data"),
            },
            attrs: SvgAttrs::default(),
            children: Vec::new(),
        })
        .collect();
    let group = SvgNode {
        primitive: SvgPrimitive::Group,
        attrs: SvgAttrs {
            view_box: Some(ViewBox::new(0.0, 0.0, graph.width, graph.height)),
            fill: Some(SvgPaint::None),
            stroke: Some(SvgPaint::Current),
            stroke_width: Some(1.5),
            stroke_linecap: Some(StrokeLineCap::Round),
            stroke_linejoin: Some(StrokeLineJoin::Round),
            ..Default::default()
        },
        children: paths,
    };
    let full = Rect {
        x: 0.0,
        y: 0.0,
        w: graph.width,
        h: graph.height,
    };
    place(ElementDef::new(Tag::Div), &full)
        .with_class("flow-graph-edges")
        .with_svg(group)
}

fn build_call_stack(pane: &FlowPane, shared: &SharedState) -> ElementDef {
    let review = pane.flow.mode == FlowMode::Review;
    let mut tree = ElementDef::new(Tag::Div).with_class("flow-tree");
    for display in pane.display_rows() {
        tree = tree.with_child(build_row(pane, display, review, shared));
        let node_id = &pane.rows[display.row].node_id;
        if pane.src_open.contains(node_id) {
            if let Some(snippet) = pane.snippet(node_id) {
                tree = tree.with_child(build_snippet(snippet, display.depth));
            }
        }
    }
    tree
}

/// The excerpt (or its failure) under an open row, indented one level
/// deeper than the row it belongs to.
/// Miller columns: the overview, then one column per node on `pane.path`.
/// Every column but the last two collapses to a strip carrying its name
/// rotated; clicking a strip refocuses that column.
fn build_panes(pane: &FlowPane, shared: &SharedState) -> ElementDef {
    let count = pane.column_count();
    let mut panes = ElementDef::new(Tag::Div).with_class("flow-panes");
    for col in 0..count {
        panes = panes.with_child(build_column(pane, col, col + 2 < count, shared));
    }
    panes
}

fn node_process_class(pane: &FlowPane, node: &Node) -> Option<String> {
    node.process
        .as_deref()
        .and_then(|id| pane.flow.process_index(id))
        .map(process_class)
}

fn build_column(pane: &FlowPane, col: usize, collapsed: bool, shared: &SharedState) -> ElementDef {
    let node = pane.column_node(col);
    let mut column = ElementDef::new(Tag::Div).with_class("flow-col");
    if collapsed {
        let mut label = ElementDef::new(Tag::Span).with_class("flow-col-vlabel");
        match node {
            Some(node) => {
                label = label.with_text(node.name.clone());
                if let Some(class) = node_process_class(pane, node) {
                    label = label.with_class(class);
                }
            }
            None => label = label.with_text(pane.flow.title.clone()),
        }
        column = column.with_class("collapsed").with_child(label);
        return dispatch_on_click(column, shared, format!("flow.focus:{col}"));
    }
    let focused = col + 1 == pane.column_count();
    if focused {
        column = column.with_class("focused");
    }
    column = column.with_child(match node {
        Some(node) => node_head(pane, node),
        None => overview_head(pane),
    });
    let chosen = pane.path.get(col).map(String::as_str);
    let cursor = if focused { pane.column_cursor } else { None };
    let review = pane.flow.mode == FlowMode::Review;
    for (section, items) in group_by_section(&pane.column_items(col)) {
        let mut list = section_block(section.label());
        for (index, item) in items {
            let Some(target) = pane.flow.node(&item.node_id) else {
                continue;
            };
            let mut button = ElementDef::new(Tag::Button).with_class("flow-item");
            if chosen == Some(item.node_id.as_str()) {
                button = button.with_class("active");
            }
            if cursor == Some(index) {
                button = button.with_class("cursor");
            }
            if review && target.status != DiffStatus::Same {
                button = button.with_class(format!("diff-{}", target.status.slug()));
            }
            let mut name = ElementDef::new(Tag::Span)
                .with_class("flow-item-name")
                .with_text(target.name.clone());
            if let Some(class) = node_process_class(pane, target) {
                name = name.with_class(class);
            }
            button = button.with_child(name);
            if let Some(carrier) = target.carrier {
                button = button.with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("flow-carrier")
                        .with_text(carrier.label()),
                );
            }
            let command = format!("flow.select:{col}:{}", item.node_id);
            list = list.with_child(dispatch_on_click(button, shared, command));
        }
        column = column.with_child(list);
    }
    if let Some(node) = node {
        if let Some(location) = &node.location {
            let mut source = section_block("source").with_child(
                ElementDef::new(Tag::Span)
                    .with_class("flow-col-loc")
                    .with_text(location.display()),
            );
            if let Some(snippet) = pane.snippet(&node.id) {
                source = source.with_child(snippet_block(snippet));
            }
            column = column.with_child(source);
        }
    }
    column
}

/// Consecutive items of one section, each with its index in the flat
/// column list (which is what the column cursor indexes).
fn group_by_section(items: &[ColumnItem]) -> Vec<(ColumnSection, Vec<(usize, &ColumnItem)>)> {
    let mut groups: Vec<(ColumnSection, Vec<(usize, &ColumnItem)>)> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        match groups.last_mut() {
            Some((section, list)) if *section == item.section => list.push((index, item)),
            _ => groups.push((item.section, vec![(index, item)])),
        }
    }
    groups
}

fn section_block(label: &str) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("flow-section")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("flow-section-label")
                .with_text(label),
        )
}

fn overview_head(pane: &FlowPane) -> ElementDef {
    let flow = &pane.flow;
    let mut head = ElementDef::new(Tag::Div)
        .with_class("flow-col-head")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("flow-col-title")
                .with_text(flow.title.clone()),
        );
    if !flow.summary.trim().is_empty() {
        head = head.with_child(
            ElementDef::new(Tag::Span)
                .with_class("flow-col-desc")
                .with_text(flow.summary.clone()),
        );
    }
    if let Some(next) = &flow.next_flow {
        head = head.with_child(
            ElementDef::new(Tag::Span)
                .with_class("flow-col-next")
                .with_text(format!("next flow: {next}")),
        );
    }
    head
}

fn node_head(pane: &FlowPane, node: &Node) -> ElementDef {
    let mut title = ElementDef::new(Tag::Span)
        .with_class("flow-col-title")
        .with_text(node.name.clone());
    if let Some(class) = node_process_class(pane, node) {
        title = title.with_class(class);
    }
    let mut line = ElementDef::new(Tag::Div)
        .with_class("flow-col-title-row")
        .with_child(title)
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("flow-tag")
                .with_class("flow-kind")
                .with_text(node.kind.as_str()),
        );
    if let Some(carrier) = node.carrier {
        line = line.with_child(
            ElementDef::new(Tag::Span)
                .with_class("flow-carrier")
                .with_text(carrier.label()),
        );
    }
    if let Some(process) = node.process.as_deref().and_then(|id| pane.flow.process(id)) {
        line = line.with_child(
            ElementDef::new(Tag::Span)
                .with_class("flow-tag")
                .with_text(process.label.clone()),
        );
    }
    let mut head = ElementDef::new(Tag::Div)
        .with_class("flow-col-head")
        .with_child(line);
    if pane.flow.mode == FlowMode::Review && node.status != DiffStatus::Same {
        head = head.with_class(format!("diff-{}", node.status.slug()));
    }
    if !node.tags.is_empty() {
        let mut tags = ElementDef::new(Tag::Div).with_class("flow-col-tags");
        for tag in &node.tags {
            tags = tags.with_child(
                ElementDef::new(Tag::Span)
                    .with_class("flow-tag")
                    .with_text(tag.clone()),
            );
        }
        head = head.with_child(tags);
    }
    if !node.description.trim().is_empty() {
        head = head.with_child(
            ElementDef::new(Tag::Span)
                .with_class("flow-col-desc")
                .with_text(node.description.clone()),
        );
    }
    if let Some(payload) = &node.payload {
        head = head.with_child(
            ElementDef::new(Tag::Span)
                .with_class("flow-col-payload")
                .with_text(payload.clone()),
        );
    }
    head
}

fn build_snippet(snippet: &Result<Snippet, SnippetError>, depth: usize) -> ElementDef {
    let mut row = ElementDef::new(Tag::Div).with_class("flow-snippet-row");
    for _ in 0..=depth {
        row = row.with_child(ElementDef::new(Tag::Span).with_class("flow-indent"));
    }
    row.with_child(snippet_block(snippet))
}

/// The excerpt itself: a path line, then gutter and tokens per line.
fn snippet_block(snippet: &Result<Snippet, SnippetError>) -> ElementDef {
    let mut body = ElementDef::new(Tag::Div).with_class("flow-snippet");
    match snippet {
        Ok(snippet) => {
            body = body.with_child(
                ElementDef::new(Tag::Div)
                    .with_class("flow-snippet-path")
                    .with_text(format!(
                        "{} \u{00B7} {}",
                        snippet.file,
                        snippet.language.as_str()
                    )),
            );
            let width = snippet.gutter_width();
            let mut in_block_comment = false;
            for (offset, line) in snippet.lines.iter().enumerate() {
                let line_no = snippet.first_line + offset as u32;
                let mut el = ElementDef::new(Tag::Div).with_class("flow-snippet-line");
                if snippet.is_highlighted(line_no) {
                    el = el.with_class("hl");
                }
                el = el.with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("flow-gutter")
                        .with_text(format!("{line_no:>width$}")),
                );
                for token in tokenize(line, snippet.language, &mut in_block_comment) {
                    el = el.with_child(
                        ElementDef::new(Tag::Span)
                            .with_class(token.kind.class())
                            .with_text(token.text),
                    );
                }
                body = body.with_child(el);
            }
        }
        Err(err) => {
            body = body.with_class("flow-snippet-failed").with_child(
                ElementDef::new(Tag::Span)
                    .with_class("flow-snippet-error")
                    .with_text(err.message()),
            );
        }
    }
    body
}

fn build_row(
    pane: &FlowPane,
    display: DisplayRow,
    review: bool,
    shared: &SharedState,
) -> ElementDef {
    let tree_row = &pane.rows[display.row];
    let node = pane.flow.node(&tree_row.node_id);
    let status = node.map(|n| n.status).unwrap_or_default();

    let mut row = ElementDef::new(Tag::Div).with_class("flow-row");
    if pane.selected_row == Some(display.row) {
        row = row.with_class("selected");
    }
    if pane.collapsed.contains(&display.row) {
        row = row.with_class("collapsed");
    }
    if review && status != DiffStatus::Same {
        row = row.with_class(format!("diff-{}", status.slug()));
    }
    row = dispatch_on_click(row, shared, format!("flow.toggle:{}", display.row));

    for _ in 0..display.depth {
        row = row.with_child(ElementDef::new(Tag::Span).with_class("flow-indent"));
    }
    if display.depth > 0 {
        row = row.with_child(
            ElementDef::new(Tag::Span)
                .with_class("flow-connector")
                .with_text("\u{2514}\u{2500}"),
        );
    }
    if review {
        row = row.with_child(
            ElementDef::new(Tag::Span)
                .with_class("flow-diff-marker")
                .with_text(status.marker().unwrap_or(" ")),
        );
    }

    let Some(node) = node else {
        return row.with_child(
            ElementDef::new(Tag::Span)
                .with_class("flow-name")
                .with_text(tree_row.node_id.clone()),
        );
    };

    let mut name = ElementDef::new(Tag::Span)
        .with_class("flow-name")
        .with_text(node.name.clone());
    if let Some(index) = node
        .process
        .as_deref()
        .and_then(|id| pane.flow.process_index(id))
    {
        name = name.with_class(process_class(index));
    }
    row = row.with_child(name);

    if let Some(carrier) = node.carrier {
        row = row.with_child(
            ElementDef::new(Tag::Span)
                .with_class("flow-carrier")
                .with_text(carrier.label()),
        );
    }
    for tag in &node.tags {
        row = row.with_child(
            ElementDef::new(Tag::Span)
                .with_class("flow-tag")
                .with_text(tag.clone()),
        );
    }
    if node.hidden_children > 0 {
        row = row.with_child(badge(format!("[+{}]", node.hidden_children)));
    }
    match tree_row.marker {
        RowMarker::Repeat { .. } => row = row.with_child(badge("\u{21A9} shown above")),
        RowMarker::Truncated => row = row.with_child(badge("\u{2026} truncated")),
        RowMarker::None => {}
    }
    if tree_row.child_count > 0 && pane.collapsed.contains(&display.row) {
        row = row.with_child(badge(format!("[{} collapsed]", tree_row.child_count)));
    }
    if !node.description.trim().is_empty() {
        row = row.with_child(
            ElementDef::new(Tag::Span)
                .with_class("flow-desc")
                .with_text(node.description.clone()),
        );
    }
    if let Some(location) = &node.location {
        row = row.with_child(
            ElementDef::new(Tag::Span)
                .with_class("flow-loc")
                .with_text(location.display()),
        );
        let mut src = ElementDef::new(Tag::Button)
            .with_class("flow-src")
            .with_text("src");
        if pane.src_open.contains(&node.id) {
            src = src.with_class("active");
        }
        row = row.with_child(dispatch_on_click(
            src,
            shared,
            format!("flow.src:{}", node.id),
        ));
    }
    row
}

fn badge(text: impl Into<String>) -> ElementDef {
    ElementDef::new(Tag::Span)
        .with_class("flow-badge")
        .with_text(text)
}

pub fn process_class(index: usize) -> String {
    format!("flow-proc-{}", index % PROCESS_PALETTE)
}

/// Two rows: processes and service state, then the eight carriers.
fn build_legend(pane: &FlowPane) -> ElementDef {
    let mut processes = ElementDef::new(Tag::Div)
        .with_class("flow-legend-row")
        .with_child(legend_label("process", true));
    for (index, process) in pane.flow.processes.iter().enumerate() {
        processes = processes.with_child(chip(&process.label).with_class(process_class(index)));
    }
    processes = processes
        .with_child(legend_label("service state", false))
        .with_child(chip("field").with_class("flow-chip-state"));
    let mut carriers = ElementDef::new(Tag::Div)
        .with_class("flow-legend-row")
        .with_child(legend_label("event carrier", true));
    for carrier in Carrier::ALL {
        carriers = carriers.with_child(
            ElementDef::new(Tag::Div)
                .with_class("flow-chip")
                .with_class(format!("flow-chip-{}", carrier.slug()))
                .with_child(svg_icon(icon_carrier(carrier)).with_class("flow-chip-icon"))
                .with_child(ElementDef::new(Tag::Span).with_text(carrier.label())),
        );
    }
    let mut legend = ElementDef::new(Tag::Div)
        .with_class("flow-legend")
        .with_child(processes)
        .with_child(carriers);
    if pane.flow.mode == FlowMode::Review {
        let mut diff = ElementDef::new(Tag::Div)
            .with_class("flow-legend-row")
            .with_child(legend_label("diff", true));
        for status in [DiffStatus::Added, DiffStatus::Removed, DiffStatus::Modified] {
            let text = format!("{} {}", status.marker().unwrap_or(""), status.slug());
            diff = diff
                .with_child(chip(&text).with_class(format!("flow-chip-diff-{}", status.slug())));
        }
        legend = legend.with_child(diff);
    }
    legend
}

fn legend_label(text: &str, first: bool) -> ElementDef {
    let mut label = ElementDef::new(Tag::Span)
        .with_class("flow-legend-label")
        .with_text(text);
    if first {
        label = label.with_class("first");
    }
    label
}

fn chip(text: &str) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("flow-chip")
        .with_child(ElementDef::new(Tag::Span).with_text(text))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::flow_explorer::test_support::fixture_path;
    use crate::state::seed_state;

    fn shared() -> SharedState {
        Arc::new(Mutex::new(seed_state()))
    }

    fn has_classes(el: &ElementDef, classes: &[&str]) -> bool {
        classes.iter().all(|c| el.classes.iter().any(|k| k == c))
    }

    fn find_all<'a>(el: &'a ElementDef, classes: &[&str], out: &mut Vec<&'a ElementDef>) {
        if has_classes(el, classes) {
            out.push(el);
        }
        for child in &el.children {
            find_all(child, classes, out);
        }
    }

    fn all<'a>(el: &'a ElementDef, classes: &[&str]) -> Vec<&'a ElementDef> {
        let mut out = Vec::new();
        find_all(el, classes, &mut out);
        out
    }

    fn first<'a>(el: &'a ElementDef, classes: &[&str]) -> Option<&'a ElementDef> {
        all(el, classes).into_iter().next()
    }

    fn text_of(el: &ElementDef) -> String {
        let mut out = String::new();
        if let ElementContent::Text(text) = &el.content {
            out.push_str(text);
        }
        for child in &el.children {
            out.push_str(&text_of(child));
        }
        out
    }

    fn pane() -> FlowPane {
        FlowPane::open(&fixture_path()).unwrap()
    }

    #[test]
    fn body_shows_title_summary_and_counts() {
        let body = build_flow_pane_body(PaneId(7), false, &pane(), &shared());
        assert!(has_classes(&body, &["pane-body", "flow-pane"]));
        assert_eq!(body.id.as_deref(), Some("flow-pane-7"));
        assert_eq!(
            text_of(first(&body, &["flow-title"]).unwrap()),
            "Send a prompt"
        );
        assert!(text_of(first(&body, &["flow-summary"]).unwrap()).starts_with("The user presses"));
        let meta = text_of(first(&body, &["flow-meta"]).unwrap());
        assert!(meta.contains("11 nodes"), "{meta}");
        assert!(meta.contains("10 edges"), "{meta}");
        assert!(meta.contains("explain"), "{meta}");
    }

    #[test]
    fn empty_summary_is_omitted() {
        let mut p = pane();
        p.flow.summary = "   ".into();
        let body = build_flow_pane_body(PaneId(1), false, &p, &shared());
        assert!(first(&body, &["flow-summary"]).is_none());
        assert!(first(&body, &["flow-title"]).is_some());
    }

    #[test]
    fn toolbar_marks_the_active_view_and_level_and_disables_unbuilt_views() {
        let body = build_flow_pane_body(PaneId(1), false, &pane(), &shared());
        let active: Vec<String> = all(&body, &["flow-tab", "active"])
            .iter()
            .map(|el| text_of(el))
            .collect();
        assert_eq!(active, vec!["call stack", "code"]);
        let disabled: Vec<String> = all(&body, &["flow-tab", "disabled"])
            .iter()
            .map(|el| text_of(el))
            .collect();
        assert_eq!(disabled, vec!["stack graph", "tree", "sequence"]);
        assert_eq!(all(&body, &["flow-tab"]).len(), 9);
        let tools: Vec<String> = all(&body, &["flow-tool"])
            .iter()
            .map(|el| text_of(el))
            .collect();
        assert_eq!(tools, vec!["expand all", "collapse all"]);
    }

    #[test]
    fn call_stack_renders_one_row_per_visible_tree_row() {
        let p = pane();
        let body = build_flow_pane_body(PaneId(1), false, &p, &shared());
        let rows = all(&body, &["flow-row"]);
        assert_eq!(rows.len(), 11);
        // Entry row: no indent, no connector, event carrier chip.
        assert!(all(rows[0], &["flow-indent"]).is_empty());
        assert!(first(rows[0], &["flow-connector"]).is_none());
        assert_eq!(text_of(first(rows[0], &["flow-carrier"]).unwrap()), "UI");
        assert!(first(rows[0], &["flow-src"]).is_none());
        // Deepest row: nine indents, a connector, a location and a src button.
        let last = rows[10];
        assert_eq!(all(last, &["flow-indent"]).len(), 9);
        assert_eq!(
            text_of(first(last, &["flow-connector"]).unwrap()),
            "\u{2514}\u{2500}"
        );
        assert!(
            text_of(first(last, &["flow-loc"]).unwrap()).ends_with("useAgentSession.ts:114-117")
        );
        assert!(first(last, &["flow-src"]).is_some());
        // Process colour comes from the process index (renderer = 1).
        let name = first(rows[1], &["flow-name"]).unwrap();
        assert!(has_classes(name, &["flow-proc-1"]), "{:?}", name.classes);
        // The RPC event carries its hidden-children badge.
        let rpc = rows
            .iter()
            .find(|r| text_of(first(r, &["flow-name"]).unwrap()) == "sessions.prompt")
            .expect("rpc row");
        assert_eq!(text_of(first(rpc, &["flow-badge"]).unwrap()), "[+1]");
        assert!(first(&body, &["flow-diff-marker"]).is_none());
    }

    #[test]
    fn collapsed_rows_hide_descendants_and_show_a_count() {
        let mut p = pane();
        assert!(p.toggle_collapsed(3));
        let body = build_flow_pane_body(PaneId(1), false, &p, &shared());
        let rows = all(&body, &["flow-row"]);
        assert_eq!(rows.len(), 4);
        assert!(has_classes(rows[3], &["collapsed"]));
        assert_eq!(
            text_of(first(rows[3], &["flow-badge"]).unwrap()),
            "[1 collapsed]"
        );
    }

    #[test]
    fn events_level_indents_by_visible_ancestors() {
        let mut p = pane();
        p.set_level(FlowLevel::Events);
        let body = build_flow_pane_body(PaneId(1), false, &p, &shared());
        let rows = all(&body, &["flow-row"]);
        assert_eq!(rows.len(), 8);
        assert_eq!(all(rows[7], &["flow-indent"]).len(), 7);
        let active: Vec<String> = all(&body, &["flow-tab", "active"])
            .iter()
            .map(|el| text_of(el))
            .collect();
        assert_eq!(active, vec!["call stack", "events"]);
    }

    #[test]
    fn open_snippets_mark_their_src_button() {
        let mut p = pane();
        assert_eq!(p.toggle_src("Editor.tsx::handleKeyDown"), Some(true));
        let body = build_flow_pane_body(PaneId(1), false, &p, &shared());
        assert_eq!(all(&body, &["flow-src", "active"]).len(), 1);
    }

    #[test]
    fn review_mode_adds_diff_classes_and_markers() {
        let mut p = pane();
        p.flow.mode = FlowMode::Review;
        p.flow.nodes[1].status = DiffStatus::Added;
        p.flow.nodes[2].status = DiffStatus::Modified;
        let body = build_flow_pane_body(PaneId(1), false, &p, &shared());
        assert_eq!(all(&body, &["flow-row", "diff-added"]).len(), 1);
        assert_eq!(all(&body, &["flow-row", "diff-modified"]).len(), 1);
        let markers: Vec<String> = all(&body, &["flow-diff-marker"])
            .iter()
            .map(|el| text_of(el))
            .collect();
        assert_eq!(markers.len(), 11);
        assert_eq!(markers[1], "+");
        assert_eq!(markers[2], "~");
        assert_eq!(markers[0], " ");
    }

    #[test]
    fn legend_lists_processes_carriers_and_service_state() {
        let body = build_flow_pane_body(PaneId(1), false, &pane(), &shared());
        let legend = first(&body, &["flow-legend"]).unwrap();
        let chips = all(legend, &["flow-chip"]);
        assert_eq!(chips.len(), 4 + 8 + 1);
        assert!(has_classes(chips[0], &["flow-proc-0"]));
        assert_eq!(text_of(chips[0]), "Human");
        assert_eq!(all(legend, &["flow-chip-icon"]).len(), 8);
        assert_eq!(text_of(chips[4]), "field");
        assert!(has_classes(chips[4], &["flow-chip-state"]));
        assert_eq!(all(legend, &["flow-legend-row"]).len(), 2);
        assert_eq!(all(&body, &["flow-toolbar-row"]).len(), 2);
    }

    #[test]
    fn open_snippet_renders_gutter_tokens_and_highlight() {
        let mut p = pane();
        assert_eq!(p.toggle_src("SessionRegistry.ts::open"), Some(true));
        assert_eq!(p.ensure_snippet("SessionRegistry.ts::open"), Some(true));
        let body = build_flow_pane_body(PaneId(1), false, &p, &shared());
        let snippet = first(&body, &["flow-snippet"]).expect("snippet under the open row");
        let lines = all(snippet, &["flow-snippet-line"]);
        let loaded = p
            .snippet("SessionRegistry.ts::open")
            .unwrap()
            .as_ref()
            .unwrap();
        assert_eq!(lines.len(), loaded.lines.len());
        assert_eq!(all(snippet, &["flow-snippet-line", "hl"]).len(), 11);
        assert_eq!(
            text_of(first(lines[0], &["flow-gutter"]).unwrap()).trim(),
            "28"
        );
        assert!(!all(snippet, &["tok-keyword"]).is_empty());
        assert_eq!(
            text_of(first(snippet, &["flow-snippet-path"]).unwrap()),
            "packages/server/src/sessions/SessionRegistry.ts \u{00B7} typescript"
        );
        // The snippet sits right after its row, indented one level deeper.
        let tree = first(&body, &["flow-tree"]).unwrap();
        let idx = tree
            .children
            .iter()
            .position(|c| has_classes(c, &["flow-snippet-row"]))
            .unwrap();
        assert!(has_classes(&tree.children[idx - 1], &["flow-row"]));
        assert_eq!(
            text_of(first(&tree.children[idx - 1], &["flow-name"]).unwrap()),
            "SessionRegistry.open"
        );
        let row_indents = all(&tree.children[idx - 1], &["flow-indent"]).len();
        assert_eq!(
            all(&tree.children[idx], &["flow-indent"]).len(),
            row_indents + 1
        );
    }

    #[test]
    fn missing_source_renders_inline_message() {
        let mut p = pane();
        p.flow.nodes[1].location.as_mut().unwrap().file = "gone/Missing.tsx".into();
        assert_eq!(p.toggle_src("Editor.tsx::handleKeyDown"), Some(true));
        assert_eq!(p.ensure_snippet("Editor.tsx::handleKeyDown"), Some(true));
        let body = build_flow_pane_body(PaneId(1), false, &p, &shared());
        assert_eq!(
            text_of(first(&body, &["flow-snippet-error"]).unwrap()),
            "source not available"
        );
        assert!(all(&body, &["flow-snippet-line"]).is_empty());
    }

    #[test]
    fn open_but_unloaded_snippet_renders_nothing() {
        let mut p = pane();
        assert_eq!(p.toggle_src("SessionRegistry.ts::open"), Some(true));
        let body = build_flow_pane_body(PaneId(1), false, &p, &shared());
        assert!(first(&body, &["flow-snippet"]).is_none());
        assert_eq!(all(&body, &["flow-src", "active"]).len(), 1);
    }

    fn key(key: Key) -> KeyboardEvent {
        KeyboardEvent {
            kind: KeyEventKind::Pressed,
            key,
            modifiers: Modifiers::empty(),
            text: None,
        }
    }

    fn key_mod(k: Key, modifiers: Modifiers) -> KeyboardEvent {
        KeyboardEvent {
            kind: KeyEventKind::Pressed,
            key: k,
            modifiers,
            text: None,
        }
    }

    #[test]
    fn active_flow_pane_captures_keyboard() {
        let p = pane();
        let active = build_flow_pane_body(PaneId(1), true, &p, &shared());
        assert!(active.captures_keyboard);
        assert_eq!(active.tab_index, Some(0));
        let inactive = build_flow_pane_body(PaneId(1), false, &p, &shared());
        assert!(!inactive.captures_keyboard);
    }

    #[test]
    fn key_mapping_covers_navigation_and_leaves_the_rest_alone() {
        assert_eq!(
            flow_key_command(&key(Key::ArrowDown)),
            Some("flow.select_move:1")
        );
        assert_eq!(
            flow_key_command(&key(Key::ArrowUp)),
            Some("flow.select_move:-1")
        );
        assert_eq!(
            flow_key_command(&key(Key::ArrowRight)),
            Some("flow.select_into")
        );
        assert_eq!(
            flow_key_command(&key(Key::ArrowLeft)),
            Some("flow.select_out")
        );
        assert_eq!(
            flow_key_command(&key(Key::Enter)),
            Some("flow.toggle_selected")
        );
        assert_eq!(flow_key_command(&key(Key::Home)), Some("flow.select_first"));
        assert_eq!(
            flow_key_command(&key(Key::Char('s'))),
            Some("flow.src_selected")
        );
        assert_eq!(
            flow_key_command(&key_mod(Key::Char('2'), Modifiers::CTRL)),
            Some("flow.view:panes")
        );
        assert_eq!(flow_key_command(&key(Key::Char('2'))), None);
        assert_eq!(flow_key_command(&key(Key::Char('x'))), None);
        assert_eq!(
            flow_key_command(&key_mod(Key::ArrowDown, Modifiers::CTRL)),
            None
        );
        assert_eq!(
            flow_key_command(&key_mod(Key::Char('s'), Modifiers::ALT)),
            None
        );
        assert_eq!(flow_key_command(&key(Key::Tab)), None);
    }

    #[test]
    fn handle_flow_key_moves_the_selection_on_the_active_flow_pane() {
        let mut st = seed_state();
        assert!(crate::state::dispatch(
            &mut st,
            &format!("flow.open:{}", fixture_path().display())
        ));
        let pane_id = st.active_pane.0;
        assert_eq!(
            handle_flow_key(&mut st, pane_id, &key(Key::ArrowDown)),
            Some(true)
        );
        assert_eq!(st.flows[&pane_id].selected_row, Some(0));
        assert_eq!(
            handle_flow_key(&mut st, pane_id, &key(Key::ArrowRight)),
            Some(true)
        );
        assert_eq!(st.flows[&pane_id].selected_row, Some(1));
        assert_eq!(
            handle_flow_key(&mut st, pane_id, &key(Key::Char('x'))),
            None
        );
        assert_eq!(
            handle_flow_key(&mut st, pane_id + 1000, &key(Key::ArrowDown)),
            None
        );
        let body = build_flow_pane_body(PaneId(pane_id), true, &st.flows[&pane_id], &shared());
        assert_eq!(all(&body, &["flow-row", "selected"]).len(), 1);
    }

    #[test]
    fn panes_view_renders_the_overview_column_with_entries() {
        let mut p = pane();
        p.set_view(FlowView::Panes);
        let body = build_flow_pane_body(PaneId(1), false, &p, &shared());
        assert!(first(&body, &["flow-placeholder"]).is_none());
        assert!(all(&body, &["flow-row"]).is_empty());
        assert_eq!(all(&body, &["flow-col"]).len(), 1);
        assert_eq!(all(&body, &["flow-col", "focused"]).len(), 1);
        assert!(all(&body, &["flow-col", "collapsed"]).is_empty());
        assert_eq!(
            text_of(first(&body, &["flow-col-title"]).unwrap()),
            "Send a prompt"
        );
        assert_eq!(
            text_of(first(&body, &["flow-section-label"]).unwrap()),
            "entries"
        );
        let items = all(&body, &["flow-item"]);
        assert_eq!(items.len(), 1);
        assert_eq!(
            text_of(first(items[0], &["flow-item-name"]).unwrap()),
            "Cmd/Ctrl+Enter in the composer"
        );
        assert!(all(&body, &["flow-item", "active"]).is_empty());
    }

    #[test]
    fn panes_view_collapses_older_columns_and_marks_the_path() {
        let mut p = pane();
        p.set_view(FlowView::Panes);
        p.path = vec![
            "ui.cmd-enter".into(),
            "Editor.tsx::handleKeyDown".into(),
            "AgentPane.tsx::submit".into(),
        ];
        p.column_cursor = Some(0);
        assert_eq!(p.ensure_snippet("AgentPane.tsx::submit"), Some(true));
        let body = build_flow_pane_body(PaneId(1), false, &p, &shared());
        assert_eq!(all(&body, &["flow-col"]).len(), 4);
        let strips = all(&body, &["flow-col", "collapsed"]);
        assert_eq!(strips.len(), 2, "all but the last two columns collapse");
        assert_eq!(
            text_of(first(strips[0], &["flow-col-vlabel"]).unwrap()),
            "Send a prompt"
        );
        assert_eq!(
            text_of(first(strips[1], &["flow-col-vlabel"]).unwrap()),
            "Cmd/Ctrl+Enter in the composer"
        );
        // The handleKeyDown column marks submit as chosen; the focused
        // submit column carries the cursor on its first item.
        let active = all(&body, &["flow-item", "active"]);
        assert_eq!(active.len(), 1);
        assert_eq!(
            text_of(first(active[0], &["flow-item-name"]).unwrap()),
            "Composer.submit"
        );
        assert_eq!(all(&body, &["flow-item", "cursor"]).len(), 1);
        let labels: Vec<String> = all(&body, &["flow-section-label"])
            .iter()
            .map(|e| text_of(e))
            .collect();
        assert_eq!(
            labels,
            vec!["calls", "handles", "source", "calls", "source"]
        );
        assert_eq!(
            all(&body, &["flow-snippet"]).len(),
            1,
            "only the loaded excerpt renders"
        );
        assert_eq!(all(&body, &["flow-col-loc"]).len(), 2);
        assert_eq!(all(&body, &["flow-kind"]).len(), 2);
    }

    #[test]
    fn review_fixture_shows_the_range_and_a_diff_legend() {
        use crate::flow_explorer::test_support::review_fixture_path;

        let explain = build_flow_pane_body(PaneId(1), false, &pane(), &shared());
        assert_eq!(all(&explain, &["flow-legend-row"]).len(), 2);
        assert!(first(&explain, &["flow-chip-diff-added"]).is_none());

        let mut p = FlowPane::open(&review_fixture_path()).unwrap();
        let body = build_flow_pane_body(PaneId(1), false, &p, &shared());
        let meta = text_of(first(&body, &["flow-meta"]).unwrap());
        assert!(meta.contains("review"), "{meta}");
        assert!(meta.contains("main..feat/prompt-restore"), "{meta}");
        assert_eq!(all(&body, &["flow-legend-row"]).len(), 3);
        let chips: Vec<String> = [
            "flow-chip-diff-added",
            "flow-chip-diff-removed",
            "flow-chip-diff-modified",
        ]
        .iter()
        .map(|c| text_of(first(&body, &[c]).unwrap()))
        .collect();
        assert_eq!(chips, vec!["+ added", "- removed", "~ modified"]);
        assert_eq!(all(&body, &["flow-row", "diff-removed"]).len(), 1);
        assert_eq!(all(&body, &["flow-row", "diff-modified"]).len(), 2);
        assert_eq!(all(&body, &["flow-row", "diff-added"]).len(), 1);

        // The panes view carries the same status onto items and heads.
        p.set_view(FlowView::Panes);
        p.path = vec!["ui.cmd-enter".into(), "Editor.tsx::handleKeyDown".into()];
        let body = build_flow_pane_body(PaneId(1), false, &p, &shared());
        assert_eq!(
            all(&body, &["flow-item", "diff-modified"]).len(),
            1,
            "submit in the calls list"
        );
        p.path.push("AgentPane.tsx::submit".into());
        let body = build_flow_pane_body(PaneId(1), false, &p, &shared());
        assert_eq!(all(&body, &["flow-col-head", "diff-modified"]).len(), 1);
        assert_eq!(
            all(&body, &["flow-item", "diff-removed"]).len(),
            1,
            "clearDraft in the calls list"
        );
    }

    #[test]
    fn graph_view_renders_lanes_boxes_edges_and_badges() {
        let mut p = pane();
        p.set_view(FlowView::Graph);
        let body = build_flow_pane_body(PaneId(1), false, &p, &shared());
        assert!(all(&body, &["flow-row"]).is_empty());
        assert_eq!(all(&body, &["flow-lane"]).len(), 3);
        assert_eq!(all(&body, &["flow-graph-node"]).len(), 9);
        assert_eq!(all(&body, &["flow-graph-node", "event"]).len(), 1);
        let badges: Vec<String> = all(&body, &["flow-graph-badge"])
            .iter()
            .map(|e| text_of(e))
            .collect();
        assert_eq!(badges, vec!["1", "2", "3"]);
        let labels = all(&body, &["flow-graph-edge-label"]);
        assert_eq!(labels.len(), 3);
        assert_eq!(
            text_of(labels[0]),
            "UI \u{00B7} Cmd/Ctrl+Enter in the composer"
        );
        let edges = first(&body, &["flow-graph-edges"]).unwrap();
        assert!(matches!(edges.content, ElementContent::Svg(_)));
        let crumbs = all(&body, &["flow-crumb"]);
        assert_eq!(crumbs.len(), 1);
        assert!(has_classes(crumbs[0], &["current"]));
        let depth_tabs = all(&body, &["flow-depth"]);
        assert_eq!(depth_tabs.len(), 4);
        assert!(has_classes(depth_tabs[3], &["active"]));
        let lanes: Vec<String> = all(&body, &["flow-lane-pill"])
            .iter()
            .map(|e| text_of(e))
            .collect();
        assert_eq!(lanes, vec!["Human", "Renderer", "Main process"]);
    }

    #[test]
    fn graph_zoom_adds_crumbs_and_shrinks_the_canvas() {
        let mut p = pane();
        p.set_view(FlowView::Graph);
        assert!(p.graph_zoom("main.ts::RPCHandler.upgrade"));
        assert!(p.graph_set_depth(DepthFilter::One));
        let body = build_flow_pane_body(PaneId(1), false, &p, &shared());
        let crumbs: Vec<String> = all(&body, &["flow-crumb"])
            .iter()
            .map(|e| text_of(e))
            .collect();
        assert_eq!(crumbs, vec!["Send a prompt", "RPCHandler.upgrade(port1)"]);
        assert_eq!(all(&body, &["flow-graph-node"]).len(), 2);
        assert!(
            all(&body, &["flow-graph-badge"]).is_empty(),
            "no event within one hop"
        );
        assert_eq!(all(&body, &["flow-lane"]).len(), 1);
        p.graph_crumbs = vec!["useAgentSession.ts::invalidateQueries".into()];
        let body = build_flow_pane_body(PaneId(1), false, &p, &shared());
        assert_eq!(all(&body, &["flow-graph-node"]).len(), 1);
        assert!(first(&body, &["flow-placeholder"]).is_none());
        p.graph_crumbs = vec!["nope".into()];
        let body = build_flow_pane_body(PaneId(1), false, &p, &shared());
        assert!(first(&body, &["flow-placeholder"]).is_some());
    }
}
