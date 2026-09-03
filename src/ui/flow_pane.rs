//! Flow Explorer pane body: header, view/level toolbar, the call-stack
//! tree and the legend. Every interaction dispatches a `flow.*` command,
//! so clicks, keys, tests and `TM_STARTUP_DISPATCH` share one path.

use unshit::core::element::*;
use unshit::core::event::{Event, EventType, Key, KeyEventKind, KeyboardEvent, Modifiers};

use crate::flow_explorer::{
    tokenize, Carrier, DiffStatus, DisplayRow, FlowLevel, FlowMode, FlowPane, FlowView, RowMarker,
    Snippet, SnippetError,
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
        FlowView::Panes | FlowView::Graph => body.with_child(build_placeholder(pane.view)),
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

fn build_placeholder(view: FlowView) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("flow-tree")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("flow-placeholder")
                .with_text(format!("{} view is not built yet", view.as_str())),
        )
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
fn build_snippet(snippet: &Result<Snippet, SnippetError>, depth: usize) -> ElementDef {
    let mut row = ElementDef::new(Tag::Div).with_class("flow-snippet-row");
    for _ in 0..=depth {
        row = row.with_child(ElementDef::new(Tag::Span).with_class("flow-indent"));
    }
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
    row.with_child(body)
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
    ElementDef::new(Tag::Div)
        .with_class("flow-legend")
        .with_child(processes)
        .with_child(carriers)
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
    fn other_views_render_a_placeholder_instead_of_rows() {
        let mut p = pane();
        p.set_view(FlowView::Panes);
        let body = build_flow_pane_body(PaneId(1), false, &p, &shared());
        assert!(all(&body, &["flow-row"]).is_empty());
        assert_eq!(
            text_of(first(&body, &["flow-placeholder"]).unwrap()),
            "panes view is not built yet"
        );
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
}
