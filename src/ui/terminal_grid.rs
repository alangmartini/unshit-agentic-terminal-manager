use unshit::core::element::*;
use unshit::core::event::{Event, EventType};

use crate::state::{
    mutate_close_pane, mutate_split_down, mutate_split_right, mutate_with, Pane,
    PaneId, SharedState, UiSnapshot,
};
use crate::ui::icons::*;

pub fn build_terminal_grid(
    state: &UiSnapshot,
    shared: &SharedState,
    grids: &std::collections::HashMap<u32, unshit::core::cell_grid::CellGrid>,
) -> ElementDef {
    let mut grid_el = ElementDef::new(Tag::Div)
        .with_class("terminal-grid")
        .with_id("terminal-grid");

    for (row_idx, row) in state.panes.iter().enumerate() {
        let mut row_el = ElementDef::new(Tag::Div).with_class("pane-row");
        for (col_idx, pane) in row.iter().enumerate() {
            let is_active = pane.id == state.active_pane;
            // Add resizer between panes (except before the first one).
            if col_idx > 0 {
                row_el = row_el.with_child(
                    ElementDef::new(Tag::Div).with_class("pane-resizer"),
                );
            }
            row_el = row_el.with_child(build_pane(pane, is_active, shared, grids));
        }
        if row_idx > 0 {
            grid_el = grid_el.with_child(
                ElementDef::new(Tag::Div).with_class("pane-resizer").with_class("vertical"),
            );
        }
        grid_el = grid_el.with_child(row_el);
    }

    grid_el
}

fn build_pane(
    pane: &Pane,
    is_active: bool,
    shared: &SharedState,
    grids: &std::collections::HashMap<u32, unshit::core::cell_grid::CellGrid>,
) -> ElementDef {
    let mut container = ElementDef::new(Tag::Div).with_class("pane");
    if is_active {
        container = container.with_class("active");
    }
    let activate_state = shared.clone();
    let pane_id = pane.id;
    container = container.on_click(move || {
        mutate_with(&activate_state, |st| {
            st.active_pane = pane_id;
        });
    });

    let body = build_pane_body(pane.id, is_active, shared, grids);
    container.with_child(build_pane_header(pane, shared)).with_child(body)
}

fn build_pane_header(pane: &Pane, shared: &SharedState) -> ElementDef {
    let meta = format!("pid {} \u{00B7} {:.1}%", pane.pid, pane.cpu);
    let pane_id = pane.id;
    let split_h_state = shared.clone();
    let split_v_state = shared.clone();
    let close_state = shared.clone();
    ElementDef::new(Tag::Div)
        .with_class("pane-header")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("pane-header-left")
                .with_child(ElementDef::new(Tag::Span).with_class("pane-status-dot"))
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("pane-title")
                        .with_text(pane.title.clone()),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("pane-subtitle")
                        .with_text(format!("\u{00B7} {}", pane.subtitle)),
                ),
        )
        .with_child(ElementDef::new(Tag::Div).with_class("pane-meta").with_text(meta))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("pane-header-right")
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("pane-action")
                        .with_child(svg_icon(icon_search())),
                )
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("pane-action")
                        .on_click(move || {
                            mutate_with(&split_h_state, |st| mutate_split_right(st, pane_id));
                        })
                        .with_child(svg_icon(icon_split_h())),
                )
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("pane-action")
                        .on_click(move || {
                            mutate_with(&split_v_state, |st| mutate_split_down(st, pane_id));
                        })
                        .with_child(svg_icon(icon_split_v())),
                )
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("pane-action")
                        .with_class("danger")
                        .on_click(move || {
                            mutate_with(&close_state, |st| mutate_close_pane(st, pane_id));
                        })
                        .with_child(svg_icon(icon_close())),
                ),
        )
}

fn build_pane_body(
    pane_id: PaneId,
    is_active: bool,
    shared: &SharedState,
    grids: &std::collections::HashMap<u32, unshit::core::cell_grid::CellGrid>,
) -> ElementDef {
    let mut body = ElementDef::new(Tag::Div).with_class("pane-body");

    if let Some(grid) = grids.get(&pane_id.0) {
        // Real terminal grid rendering.
        let mut grid_el = ElementDef::new(Tag::Div)
            .with_class("terminal-content")
            .with_grid(grid.clone())
            .with_persistent_buffer(true);

        if is_active {
            grid_el = grid_el.captures_keyboard(true);

            // Register keyboard capture handler to send input to PTY.
            let kbd_shared = shared.clone();
            let kbd_pane_id = pane_id;
            grid_el = grid_el.on(
                EventType::KeyboardCapture,
                move |event: &Event| -> Option<Box<dyn std::any::Any>> {
                    if let Event::Keyboard(kb) = event {
                        if let Some(bytes) = crate::terminal::keys::encode_key(kb) {
                            mutate_with(&kbd_shared, |st| {
                                let _ = st.pty_manager.write(kbd_pane_id.0, &bytes);
                            });
                        }
                    }
                    None
                },
            );

            // Register resize handler to update PTY dimensions.
            let resize_shared = shared.clone();
            let resize_pane_id = pane_id;
            grid_el = grid_el.on_resize(move |w, h| {
                // Estimate character dimensions: ~8px wide, ~16px tall for monospace.
                let cols = (w / 8.0).max(1.0) as u16;
                let rows = (h / 16.0).max(1.0) as u16;
                mutate_with(&resize_shared, |st| {
                    st.pty_manager.resize(resize_pane_id.0, cols, rows);
                    if let Some(terminal) = st.terminals.get_mut(&resize_pane_id.0) {
                        terminal.resize(rows as usize, cols as usize);
                    }
                });
            });
        }

        body = body.with_child(grid_el);
    } else {
        // Fallback: show a simple prompt placeholder.
        body = body.with_child(
            ElementDef::new(Tag::Div)
                .with_class("term-line")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("term-prompt")
                        .with_text("\u{276F} "),
                )
                .with_child(ElementDef::new(Tag::Span).with_class("term-cursor")),
        );
    }

    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{seed_state, Pane, PaneId};
    use std::sync::{Arc, Mutex};
    use unshit::core::cell_grid::CellGrid;

    fn make_shared() -> SharedState {
        Arc::new(Mutex::new(seed_state()))
    }

    fn make_snapshot() -> UiSnapshot {
        seed_state().ui_snapshot()
    }

    fn make_pane(id: u32, title: &str) -> Pane {
        Pane {
            id: PaneId(id),
            title: title.to_string(),
            subtitle: "bash".to_string(),
            pid: 1234,
            cpu: 5.3,
        }
    }

    // -- build_terminal_grid: single pane ---------------------------------------

    #[test]
    fn terminal_grid_single_pane_has_correct_structure() {
        let snap = make_snapshot(); // default: single pane
        let shared = make_shared();
        let grids = std::collections::HashMap::new();
        let el = build_terminal_grid(&snap, &shared, &grids);
        assert!(el.classes.contains(&"terminal-grid".to_string()));
        assert_eq!(el.id.as_deref(), Some("terminal-grid"));
        // Single pane = one row, no vertical resizers
        assert_eq!(el.children.len(), 1);
        let row = &el.children[0];
        assert!(row.classes.contains(&"pane-row".to_string()));
        // Single pane in the row, no horizontal resizers
        assert_eq!(row.children.len(), 1);
    }

    // -- build_terminal_grid: 2x2 layout ----------------------------------------

    #[test]
    fn terminal_grid_2x2_has_resizers() {
        let mut state = seed_state();
        state.panes = vec![
            vec![make_pane(1, "a"), make_pane(2, "b")],
            vec![make_pane(3, "c"), make_pane(4, "d")],
        ];
        state.active_pane = PaneId(1);
        let snap = state.ui_snapshot();
        let shared = make_shared();
        let grids = std::collections::HashMap::new();
        let el = build_terminal_grid(&snap, &shared, &grids);

        // 2 rows + 1 vertical resizer between them = 3 children
        assert_eq!(el.children.len(), 3);

        // The vertical resizer is at index 1
        let v_resizer = &el.children[1];
        assert!(v_resizer.classes.contains(&"pane-resizer".to_string()));
        assert!(v_resizer.classes.contains(&"vertical".to_string()));

        // Each row has 2 panes + 1 horizontal resizer = 3 children
        let row0 = &el.children[0];
        assert_eq!(row0.children.len(), 3);
        let h_resizer = &row0.children[1];
        assert!(h_resizer.classes.contains(&"pane-resizer".to_string()));

        let row1 = &el.children[2];
        assert_eq!(row1.children.len(), 3);
    }

    // -- build_pane: active vs inactive -----------------------------------------

    #[test]
    fn pane_active_has_active_class() {
        let pane = make_pane(1, "shell");
        let shared = make_shared();
        let grids = std::collections::HashMap::new();
        let el = build_pane(&pane, true, &shared, &grids);
        assert!(el.classes.contains(&"pane".to_string()));
        assert!(el.classes.contains(&"active".to_string()));
    }

    #[test]
    fn pane_inactive_lacks_active_class() {
        let pane = make_pane(1, "shell");
        let shared = make_shared();
        let grids = std::collections::HashMap::new();
        let el = build_pane(&pane, false, &shared, &grids);
        assert!(el.classes.contains(&"pane".to_string()));
        assert!(!el.classes.contains(&"active".to_string()));
    }

    #[test]
    fn pane_has_header_and_body() {
        let pane = make_pane(1, "shell");
        let shared = make_shared();
        let grids = std::collections::HashMap::new();
        let el = build_pane(&pane, true, &shared, &grids);
        assert_eq!(el.children.len(), 2);
        assert!(el.children[0].classes.contains(&"pane-header".to_string()));
        assert!(el.children[1].classes.contains(&"pane-body".to_string()));
    }

    #[test]
    fn pane_has_click_handler() {
        let pane = make_pane(1, "shell");
        let shared = make_shared();
        let grids = std::collections::HashMap::new();
        let el = build_pane(&pane, false, &shared, &grids);
        assert!(el.on_click.is_some());
    }

    // -- build_pane_header ------------------------------------------------------

    #[test]
    fn pane_header_has_correct_class() {
        let pane = make_pane(42, "zsh");
        let shared = make_shared();
        let el = build_pane_header(&pane, &shared);
        assert!(el.classes.contains(&"pane-header".to_string()));
    }

    #[test]
    fn pane_header_has_three_sections() {
        let pane = make_pane(42, "zsh");
        let shared = make_shared();
        let el = build_pane_header(&pane, &shared);
        // left, meta, right
        assert_eq!(el.children.len(), 3);
        assert!(el.children[0].classes.contains(&"pane-header-left".to_string()));
        assert!(el.children[1].classes.contains(&"pane-meta".to_string()));
        assert!(el.children[2].classes.contains(&"pane-header-right".to_string()));
    }

    #[test]
    fn pane_header_meta_shows_pid_and_cpu() {
        let pane = make_pane(42, "zsh");
        let shared = make_shared();
        let el = build_pane_header(&pane, &shared);
        let meta = &el.children[1];
        // meta text should contain "pid 1234" and "5.3%"
        if let unshit::core::element::ElementContent::Text(ref text) = meta.content {
            assert!(text.contains("1234"), "expected pid in meta, got: {}", text);
            assert!(text.contains("5.3"), "expected cpu in meta, got: {}", text);
        } else {
            panic!("expected text content in pane-meta");
        }
    }

    #[test]
    fn pane_header_right_has_action_buttons() {
        let pane = make_pane(1, "shell");
        let shared = make_shared();
        let el = build_pane_header(&pane, &shared);
        let right = &el.children[2];
        // search, split_h, split_v, close = 4 buttons
        assert_eq!(right.children.len(), 4);
        // Last button (close) should have "danger" class
        assert!(right.children[3].classes.contains(&"danger".to_string()));
    }

    // -- build_pane_body: with grid ---------------------------------------------

    #[test]
    fn pane_body_with_grid_renders_terminal_content() {
        let shared = make_shared();
        let mut grids = std::collections::HashMap::new();
        grids.insert(1, CellGrid::new(24, 80));
        let el = build_pane_body(PaneId(1), true, &shared, &grids);
        assert!(el.classes.contains(&"pane-body".to_string()));
        assert_eq!(el.children.len(), 1);
        let grid_el = &el.children[0];
        assert!(grid_el.classes.contains(&"terminal-content".to_string()));
        assert!(grid_el.persistent_buffer);
    }

    #[test]
    fn pane_body_with_grid_active_captures_keyboard() {
        let shared = make_shared();
        let mut grids = std::collections::HashMap::new();
        grids.insert(1, CellGrid::new(24, 80));
        let el = build_pane_body(PaneId(1), true, &shared, &grids);
        let grid_el = &el.children[0];
        assert!(grid_el.captures_keyboard);
    }

    #[test]
    fn pane_body_with_grid_inactive_does_not_capture_keyboard() {
        let shared = make_shared();
        let mut grids = std::collections::HashMap::new();
        grids.insert(1, CellGrid::new(24, 80));
        let el = build_pane_body(PaneId(1), false, &shared, &grids);
        let grid_el = &el.children[0];
        assert!(!grid_el.captures_keyboard);
    }

    // -- build_pane_body: without grid (fallback) -------------------------------

    #[test]
    fn pane_body_without_grid_shows_fallback() {
        let shared = make_shared();
        let grids = std::collections::HashMap::new(); // no grid for pane 1
        let el = build_pane_body(PaneId(1), true, &shared, &grids);
        assert!(el.classes.contains(&"pane-body".to_string()));
        assert_eq!(el.children.len(), 1);
        let fallback = &el.children[0];
        assert!(fallback.classes.contains(&"term-line".to_string()));
        // Should have prompt and cursor children
        assert_eq!(fallback.children.len(), 2);
        assert!(fallback.children[0].classes.contains(&"term-prompt".to_string()));
        assert!(fallback.children[1].classes.contains(&"term-cursor".to_string()));
    }

    #[test]
    fn pane_body_without_grid_inactive_also_shows_fallback() {
        let shared = make_shared();
        let grids = std::collections::HashMap::new();
        let el = build_pane_body(PaneId(99), false, &shared, &grids);
        assert_eq!(el.children.len(), 1);
        assert!(el.children[0].classes.contains(&"term-line".to_string()));
    }

    // -- closure invocation tests (cover on_click/on_resize bodies) ------------

    #[test]
    fn pane_click_sets_active_pane() {
        let shared = make_shared();
        let pane = make_pane(42, "shell");
        let grids = std::collections::HashMap::new();
        let el = build_pane(&pane, false, &shared, &grids);
        (el.on_click.as_ref().unwrap())();
        assert_eq!(shared.lock().unwrap().active_pane, PaneId(42));
    }

    #[test]
    fn pane_header_split_h_has_click_handler() {
        let shared = make_shared();
        let pane = make_pane(1, "shell");
        let el = build_pane_header(&pane, &shared);
        let right = &el.children[2];
        // split_h is the second action button (index 1)
        let split_h = &right.children[1];
        assert!(split_h.on_click.is_some());
        assert!(split_h.classes.contains(&"pane-action".to_string()));
    }

    #[test]
    fn pane_header_split_v_has_click_handler() {
        let shared = make_shared();
        let pane = make_pane(1, "shell");
        let el = build_pane_header(&pane, &shared);
        let right = &el.children[2];
        // split_v is the third action button (index 2)
        let split_v = &right.children[2];
        assert!(split_v.on_click.is_some());
        assert!(split_v.classes.contains(&"pane-action".to_string()));
    }

    #[test]
    fn pane_header_close_has_click_handler_and_danger_class() {
        let shared = make_shared();
        let pane = make_pane(1, "shell");
        let el = build_pane_header(&pane, &shared);
        let right = &el.children[2];
        // close is the last action button (index 3)
        let close_btn = &right.children[3];
        assert!(close_btn.classes.contains(&"danger".to_string()));
        assert!(close_btn.classes.contains(&"pane-action".to_string()));
        assert!(close_btn.on_click.is_some());
    }

    #[test]
    fn pane_body_active_grid_has_keyboard_handler() {
        let shared = make_shared();
        let mut grids = std::collections::HashMap::new();
        grids.insert(1, CellGrid::new(24, 80));
        let el = build_pane_body(PaneId(1), true, &shared, &grids);
        let grid_el = &el.children[0];
        // Should have event handlers registered (KeyboardCapture)
        assert!(!grid_el.handlers.is_empty());
    }

    #[test]
    fn pane_body_active_grid_has_resize_handler() {
        let shared = make_shared();
        let mut grids = std::collections::HashMap::new();
        grids.insert(1, CellGrid::new(24, 80));
        let el = build_pane_body(PaneId(1), true, &shared, &grids);
        let grid_el = &el.children[0];
        assert!(grid_el.on_resize.is_some());
    }

    #[test]
    fn pane_body_inactive_grid_has_no_keyboard_handler() {
        let shared = make_shared();
        let mut grids = std::collections::HashMap::new();
        grids.insert(1, CellGrid::new(24, 80));
        let el = build_pane_body(PaneId(1), false, &shared, &grids);
        let grid_el = &el.children[0];
        assert!(grid_el.handlers.is_empty());
        assert!(grid_el.on_resize.is_none());
    }

    #[test]
    fn pane_body_resize_handler_invocation() {
        let shared = make_shared();
        let mut grids = std::collections::HashMap::new();
        grids.insert(1, CellGrid::new(24, 80));
        let el = build_pane_body(PaneId(1), true, &shared, &grids);
        let grid_el = &el.children[0];
        let resize_fn = grid_el.on_resize.as_ref().unwrap();
        // Invoke with a 640x384 area (should yield 80 cols, 24 rows)
        (resize_fn)(640.0, 384.0);
        // The resize handler should not panic and should work
    }

    #[test]
    fn pane_header_left_has_status_dot_title_subtitle() {
        let pane = make_pane(1, "zsh");
        let shared = make_shared();
        let el = build_pane_header(&pane, &shared);
        let left = &el.children[0];
        assert!(left.classes.contains(&"pane-header-left".to_string()));
        assert_eq!(left.children.len(), 3);
        assert!(left.children[0].classes.contains(&"pane-status-dot".to_string()));
        assert!(left.children[1].classes.contains(&"pane-title".to_string()));
        assert!(left.children[2].classes.contains(&"pane-subtitle".to_string()));
    }

    #[test]
    fn terminal_grid_with_three_cols() {
        let mut state = seed_state();
        state.panes = vec![vec![
            make_pane(1, "a"),
            make_pane(2, "b"),
            make_pane(3, "c"),
        ]];
        state.active_pane = PaneId(1);
        let snap = state.ui_snapshot();
        let shared = make_shared();
        let grids = std::collections::HashMap::new();
        let el = build_terminal_grid(&snap, &shared, &grids);
        // 1 row, no vertical resizers
        assert_eq!(el.children.len(), 1);
        let row = &el.children[0];
        // 3 panes + 2 resizers = 5
        assert_eq!(row.children.len(), 5);
    }
}
