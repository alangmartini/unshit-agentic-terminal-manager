//! Flow Explorer pane body: the flow header plus the active view (call
//! stack, Miller columns, graph). This slice renders the header; the views
//! land in the following slices.

use unshit::core::element::*;

use crate::flow_explorer::FlowPane;
use crate::state::PaneId;

pub fn build_flow_pane_body(pane_id: PaneId, pane: &FlowPane) -> ElementDef {
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
    header = header.with_child(
        ElementDef::new(Tag::Span)
            .with_class("flow-meta")
            .with_text(meta_line(pane)),
    );

    ElementDef::new(Tag::Div)
        .with_class("pane-body")
        .with_class("flow-pane")
        .with_id(format!("flow-pane-{}", pane_id.0))
        .with_child(header)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_explorer::test_support::fixture_path;

    fn find_class<'a>(el: &'a ElementDef, class: &str) -> Option<&'a ElementDef> {
        if el.classes.iter().any(|c| c == class) {
            return Some(el);
        }
        el.children
            .iter()
            .find_map(|child| find_class(child, class))
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

    #[test]
    fn body_shows_title_summary_and_counts() {
        let pane = FlowPane::open(&fixture_path()).unwrap();
        let body = build_flow_pane_body(PaneId(7), &pane);
        assert!(body.classes.iter().any(|c| c == "pane-body"));
        assert!(body.classes.iter().any(|c| c == "flow-pane"));
        assert_eq!(
            text_of(find_class(&body, "flow-title").unwrap()),
            "Send a prompt"
        );
        assert!(text_of(find_class(&body, "flow-summary").unwrap()).starts_with("The user presses"));
        let meta = text_of(find_class(&body, "flow-meta").unwrap());
        assert!(meta.contains("11 nodes"), "{meta}");
        assert!(meta.contains("10 edges"), "{meta}");
        assert!(meta.contains("explain"), "{meta}");
    }

    #[test]
    fn empty_summary_is_omitted() {
        let mut pane = FlowPane::open(&fixture_path()).unwrap();
        pane.flow.summary = "   ".into();
        let body = build_flow_pane_body(PaneId(1), &pane);
        assert!(find_class(&body, "flow-summary").is_none());
        assert!(find_class(&body, "flow-title").is_some());
    }
}
