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
