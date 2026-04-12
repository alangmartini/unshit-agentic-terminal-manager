use unshit::core::element::*;
use unshit::core::event::{Event, EventType};

use crate::state::{
    mutate_close_pane, mutate_split_down, mutate_split_right, mutate_with, Pane, PaneId,
    SharedState, UiSnapshot,
};
use crate::ui::icons::*;

/// Returns `true` when the pane grid contains exactly one pane (one row with
/// one column). In that case the tab bar already displays the pane title and
/// subtitle, so the pane header can omit them to avoid visual duplication.
fn is_single_pane(panes: &[Vec<Pane>]) -> bool {
    panes.len() == 1 && panes[0].len() == 1
}

pub fn build_terminal_grid(
    state: &UiSnapshot,
    shared: &SharedState,
    grids: &std::collections::HashMap<u32, unshit::core::cell_grid::CellGrid>,
) -> ElementDef {
    let mut grid_el = ElementDef::new(Tag::Div)
        .with_class("terminal-grid")
        .with_id("terminal-grid");

    let single_pane = is_single_pane(&state.panes);

    for (row_idx, row) in state.panes.iter().enumerate() {
        let mut row_el = ElementDef::new(Tag::Div).with_class("pane-row");
        for (col_idx, pane) in row.iter().enumerate() {
            let is_active = pane.id == state.active_pane;
            // Add resizer between panes (except before the first one).
            if col_idx > 0 {
                row_el = row_el.with_child(ElementDef::new(Tag::Div).with_class("pane-resizer"));
            }
            row_el = row_el.with_child(build_pane(pane, is_active, single_pane, shared, grids));
        }
        if row_idx > 0 {
            grid_el = grid_el.with_child(
                ElementDef::new(Tag::Div)
                    .with_class("pane-resizer")
                    .with_class("vertical"),
            );
        }
        grid_el = grid_el.with_child(row_el);
    }

    grid_el
}

fn build_pane(
    pane: &Pane,
    is_active: bool,
    single_pane: bool,
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
    container
        .with_child(build_pane_header(pane, single_pane, shared))
        .with_child(body)
}

fn build_pane_header(pane: &Pane, single_pane: bool, shared: &SharedState) -> ElementDef {
    let meta = format!("pid {} \u{00B7} {:.1}%", pane.pid, pane.cpu);
    let pane_id = pane.id;
    let split_h_state = shared.clone();
    let split_v_state = shared.clone();
    let close_state = shared.clone();
    let mut header = ElementDef::new(Tag::Div).with_class("pane-header");

    // When there is only a single pane the tab bar already shows the title and
    // subtitle, so we omit the left section to avoid visual duplication.
    if !single_pane {
        header = header.with_child(
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
        );
    }

    header
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("pane-meta")
                .with_text(meta),
        )
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

        // A tab_index is required so the element is focusable; without it the
        // framework ignores click-to-focus and keyboard events never arrive.
        grid_el = grid_el.with_tab_index(0);

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
            // Prefer the renderer-computed pending resize (exact), fall
            // back to global cell metrics, then to hardcoded estimates.
            let resize_shared = shared.clone();
            let resize_pane_id = pane_id;
            grid_el = grid_el.on_resize(move |w, h| {
                use unshit::core::cell_grid::CellGrid;

                mutate_with(&resize_shared, |st| {
                    st.last_grid_width = w;
                    st.last_grid_height = h;

                    // Use the renderer's published cell metrics when available.
                    // On the first frame, metrics may be 0 because on_resize
                    // fires before the render pass. The on_cell_metrics callback
                    // and blink subscription handle the initial correction.
                    let cell_w = CellGrid::global_cell_w();
                    let cell_h = CellGrid::global_cell_h();
                    if cell_w > 0.0 && cell_h > 0.0 {
                        let cols = (w / cell_w).max(1.0) as u16;
                        let rows = (h / cell_h).max(1.0) as u16;
                        st.pty_manager.resize(resize_pane_id.0, cols, rows);
                        if let Some(terminal) = st.terminals.get_mut(&resize_pane_id.0) {
                            terminal.resize(rows as usize, cols as usize);
                        }
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
    use std::sync::{Arc, Mutex};
    use unshit::core::cell_grid::CellGrid;

    /// Build a minimal shared state for testing. Does not spawn any real PTY.
    fn test_shared() -> SharedState {
        Arc::new(Mutex::new(crate::state::seed_state()))
    }

    fn find_terminal_content(def: &ElementDef) -> Option<&ElementDef> {
        if def.classes.iter().any(|c| c == "terminal-content") {
            return Some(def);
        }
        for child in &def.children {
            if let Some(found) = find_terminal_content(child) {
                return Some(found);
            }
        }
        None
    }

    /// Regression test: terminal-content must have tab_index so the framework
    /// treats it as focusable. Without this, click-to-focus never fires and
    /// keyboard events are silently dropped.
    #[test]
    fn terminal_content_has_tab_index_for_focus() {
        let shared = test_shared();
        let pane_id = PaneId(1);
        let grid = CellGrid::new(24, 80);
        let mut grids = std::collections::HashMap::new();
        grids.insert(pane_id.0, grid);

        let body = build_pane_body(pane_id, true, &shared, &grids);
        let content = find_terminal_content(&body)
            .expect("terminal-content element should exist when grid is present");
        assert_eq!(
            content.tab_index,
            Some(0),
            "terminal-content must have tab_index=0 to be focusable"
        );
    }

    /// The active pane's terminal-content must have captures_keyboard enabled
    /// so keystrokes are forwarded to the PTY instead of handled as shortcuts.
    #[test]
    fn active_pane_captures_keyboard() {
        let shared = test_shared();
        let pane_id = PaneId(1);
        let grid = CellGrid::new(24, 80);
        let mut grids = std::collections::HashMap::new();
        grids.insert(pane_id.0, grid);

        let body = build_pane_body(pane_id, true, &shared, &grids);
        let content = find_terminal_content(&body).expect("terminal-content element should exist");
        assert!(
            content.captures_keyboard,
            "active pane terminal-content must capture keyboard"
        );
    }

    /// An inactive pane should still be focusable (tab_index set) but must
    /// NOT capture the keyboard so that shortcuts keep working.
    #[test]
    fn inactive_pane_does_not_capture_keyboard() {
        let shared = test_shared();
        let pane_id = PaneId(1);
        let grid = CellGrid::new(24, 80);
        let mut grids = std::collections::HashMap::new();
        grids.insert(pane_id.0, grid);

        let body = build_pane_body(pane_id, false, &shared, &grids);
        let content = find_terminal_content(&body).expect("terminal-content element should exist");
        assert_eq!(
            content.tab_index,
            Some(0),
            "inactive pane terminal-content should still be focusable"
        );
        assert!(
            !content.captures_keyboard,
            "inactive pane must not capture keyboard"
        );
    }

    /// Active pane must register an on_resize handler so the PTY dimensions
    /// stay in sync with the visible grid area.
    #[test]
    fn active_pane_registers_resize_handler() {
        let shared = test_shared();
        let pane_id = PaneId(1);
        let grid = CellGrid::new(24, 80);
        let mut grids = std::collections::HashMap::new();
        grids.insert(pane_id.0, grid);

        let body = build_pane_body(pane_id, true, &shared, &grids);
        let content = find_terminal_content(&body).expect("terminal-content element should exist");
        assert!(
            content.on_resize.is_some(),
            "active pane terminal-content must have a resize handler"
        );
    }

    /// The resize handler should prefer renderer-computed pending resize
    /// when available (issue #5 fix). When CellGrid::publish_pending_resize
    /// was called before the on_resize handler fires, take_pending_resize
    /// should return the exact dimensions.
    #[test]
    fn pending_resize_round_trips_through_cell_grid() {
        // Publish a pending resize to simulate the renderer computing
        // grid dimensions atomically.
        CellGrid::publish_pending_resize(100, 30);
        let result = CellGrid::take_pending_resize();
        assert_eq!(
            result,
            Some((100, 30)),
            "take_pending_resize should return the exact cols/rows published by the renderer"
        );

        // After take, should be cleared.
        let second = CellGrid::take_pending_resize();
        assert!(
            second.is_none(),
            "pending resize should be consumed after take"
        );
    }

    // -----------------------------------------------------------------------
    // Helpers for pane-header deduplication tests
    // -----------------------------------------------------------------------

    /// Recursively search the element tree for a node whose classes contain
    /// `class_name`. Returns `true` when at least one match is found.
    fn tree_has_class(def: &ElementDef, class_name: &str) -> bool {
        if def.classes.iter().any(|c| c == class_name) {
            return true;
        }
        def.children.iter().any(|c| tree_has_class(c, class_name))
    }

    /// Recursively search the element tree for any node whose text content
    /// contains `needle`.
    fn tree_has_text(def: &ElementDef, needle: &str) -> bool {
        if let unshit::core::element::ElementContent::Text(ref t) = def.content {
            if t.contains(needle) {
                return true;
            }
        }
        def.children.iter().any(|c| tree_has_text(c, needle))
    }

    fn make_pane(id: u32) -> Pane {
        Pane {
            id: PaneId(id),
            title: "shell".to_string(),
            subtitle: "bash".to_string(),
            pid: 42,
            cpu: 1.5,
        }
    }

    // -----------------------------------------------------------------------
    // Pane header deduplication: single pane hides title/subtitle
    // -----------------------------------------------------------------------

    /// When there is only one pane the tab bar already shows "shell  bash", so
    /// the pane header must NOT duplicate the title and subtitle.
    #[test]
    fn single_pane_header_omits_title_and_subtitle() {
        let shared = test_shared();
        let pane = make_pane(1);
        let header = build_pane_header(&pane, true, &shared);

        assert!(
            !tree_has_class(&header, "pane-header-left"),
            "single-pane header must not contain .pane-header-left"
        );
        assert!(
            !tree_has_class(&header, "pane-title"),
            "single-pane header must not contain .pane-title"
        );
        assert!(
            !tree_has_class(&header, "pane-subtitle"),
            "single-pane header must not contain .pane-subtitle"
        );
        // Meta and action buttons must still be present.
        assert!(
            tree_has_class(&header, "pane-meta"),
            "single-pane header must still contain .pane-meta"
        );
        assert!(
            tree_has_class(&header, "pane-header-right"),
            "single-pane header must still contain .pane-header-right"
        );
    }

    /// When there are multiple panes (split layout) every pane header must
    /// show its title and subtitle so the user can tell them apart.
    #[test]
    fn multi_pane_header_shows_title_and_subtitle() {
        let shared = test_shared();
        let pane = make_pane(1);
        let header = build_pane_header(&pane, false, &shared);

        assert!(
            tree_has_class(&header, "pane-header-left"),
            "multi-pane header must contain .pane-header-left"
        );
        assert!(
            tree_has_class(&header, "pane-title"),
            "multi-pane header must contain .pane-title"
        );
        assert!(
            tree_has_text(&header, "shell"),
            "multi-pane header must display the pane title text"
        );
        assert!(
            tree_has_text(&header, "bash"),
            "multi-pane header must display the pane subtitle text"
        );
    }

    /// The `is_single_pane` helper must return the correct value for various
    /// pane grid shapes.
    #[test]
    fn is_single_pane_detection() {
        let one = vec![vec![make_pane(1)]];
        assert!(is_single_pane(&one), "1x1 grid should be single pane");

        let two_cols = vec![vec![make_pane(1), make_pane(2)]];
        assert!(!is_single_pane(&two_cols), "1x2 grid is not single pane");

        let two_rows = vec![vec![make_pane(1)], vec![make_pane(2)]];
        assert!(!is_single_pane(&two_rows), "2x1 grid is not single pane");

        let empty: Vec<Vec<Pane>> = vec![];
        assert!(!is_single_pane(&empty), "empty grid is not single pane");
    }
}
