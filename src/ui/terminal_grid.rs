use unshit::core::element::*;
use unshit::core::event::{DragPhase, Event, EventType, Key, KeyEventKind, Modifiers};
use unshit::core::style::parse::StyleDeclaration;
use unshit::core::style::types::TransformX;

use crate::state::{
    apply_ratio_delta, mutate_close_pane, mutate_split_down, mutate_split_right, mutate_with,
    MutexExt, Pane, PaneId, ResizeDragSnapshot, SharedState, UiSnapshot, CSS_LINE_HEIGHT,
};
use crate::ui::icons::*;

const ENV_PARITY_LINE_HEIGHT: &str = "TM_PARITY_LINE_HEIGHT";
const ENV_PARITY_CONTENT_X_OFFSET: &str = "TM_PARITY_CONTENT_X_OFFSET";
const ENV_PARITY_WINDOWS_TERMINAL_COLORS: &str = "TM_PARITY_WINDOWS_TERMINAL_COLORS";
const WINDOWS_TERMINAL_PARITY_LINE_HEIGHT: f32 = 1.15;
const WINDOWS_TERMINAL_PARITY_CONTENT_X_OFFSET: f32 = 3.0;

/// Returns `true` when the pane grid contains exactly one pane (one row with
/// one column). In that case the tab bar already displays the pane title and
/// subtitle, so the pane header can omit them to avoid visual duplication.
fn is_single_pane(panes: &[Vec<Pane>]) -> bool {
    panes.len() == 1 && panes[0].len() == 1
}

fn terminal_line_height_from_values(value: Option<std::ffi::OsString>, wt_profile: bool) -> f32 {
    value
        .and_then(|v| v.into_string().ok())
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| (0.5..=2.0).contains(v))
        .unwrap_or(if wt_profile {
            WINDOWS_TERMINAL_PARITY_LINE_HEIGHT
        } else {
            CSS_LINE_HEIGHT
        })
}

fn terminal_line_height() -> f32 {
    terminal_line_height_from_values(
        std::env::var_os(ENV_PARITY_LINE_HEIGHT),
        crate::truthy_env_value(std::env::var_os(ENV_PARITY_WINDOWS_TERMINAL_COLORS)),
    )
}

fn terminal_content_x_offset_from_values(
    value: Option<std::ffi::OsString>,
    wt_profile: bool,
) -> f32 {
    value
        .and_then(|v| v.into_string().ok())
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| (-32.0..=32.0).contains(v))
        .unwrap_or(if wt_profile {
            WINDOWS_TERMINAL_PARITY_CONTENT_X_OFFSET
        } else {
            0.0
        })
}

fn terminal_content_x_offset() -> f32 {
    terminal_content_x_offset_from_values(
        std::env::var_os(ENV_PARITY_CONTENT_X_OFFSET),
        crate::truthy_env_value(std::env::var_os(ENV_PARITY_WINDOWS_TERMINAL_COLORS)),
    )
}

#[cfg(test)]
fn parity_windows_terminal_profile_from_value(value: Option<std::ffi::OsString>) -> bool {
    crate::truthy_env_value(value)
}

pub fn build_terminal_grid(
    state: &UiSnapshot,
    shared: &SharedState,
    grids: &std::collections::HashMap<u32, unshit::core::cell_grid::CellGrid>,
) -> ElementDef {
    if state.panes.is_empty() {
        return build_empty_workspace(shared);
    }

    let resize_shared = shared.clone();
    let mut grid_el = ElementDef::new(Tag::Div)
        .with_class("terminal-grid")
        .with_id("terminal-grid")
        // The full grid's dimensions drive hit-testing for pane-edge drops
        // and the container_size used by column/row resizers. Per-pane
        // `terminal-content` on_resize only sees its own subrect, so we
        // capture the true grid size here.
        .on_resize(move |w, h| {
            mutate_with(&resize_shared, |st| {
                st.last_grid_width = w;
                st.last_grid_height = h;
            });
        });

    let single_pane = is_single_pane(&state.panes);

    for (row_idx, row) in state.panes.iter().enumerate() {
        let row_ratio = state.row_ratios.get(row_idx).copied().unwrap_or(1.0);
        let mut row_el = ElementDef::new(Tag::Div)
            .with_class("pane-row")
            .with_style(StyleDeclaration::FlexGrow(row_ratio));
        for (col_idx, pane) in row.iter().enumerate() {
            let is_active = pane.id == state.active_pane;
            if col_idx > 0 {
                row_el = row_el.with_child(build_col_resizer(row_idx, col_idx, shared));
            }
            let col_ratio = state
                .col_ratios
                .get(row_idx)
                .and_then(|r| r.get(col_idx))
                .copied()
                .unwrap_or(1.0);
            let capture_keyboard = is_active && !state.settings_open;
            let pane_el = build_pane(
                pane,
                is_active,
                capture_keyboard,
                single_pane,
                state,
                shared,
                grids,
            )
            .with_style(StyleDeclaration::FlexGrow(col_ratio));
            row_el = row_el.with_child(pane_el);
        }
        if row_idx > 0 {
            grid_el = grid_el.with_child(build_row_resizer(row_idx, shared));
        }
        grid_el = grid_el.with_child(row_el);
    }

    grid_el
}

fn build_empty_workspace(shared: &SharedState) -> ElementDef {
    let click_shared = shared.clone();
    let button = ElementDef::new(Tag::Button)
        .with_class("empty-workspace-new-terminal")
        .with_tab_index(0)
        .on_click(move || {
            mutate_with(&click_shared, |st| {
                crate::state::dispatch(st, "tab.new");
            });
        })
        .with_text("New terminal".to_string());
    ElementDef::new(Tag::Div)
        .with_class("terminal-grid")
        .with_class("empty")
        .with_id("terminal-grid")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("empty-workspace")
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("empty-workspace-message")
                        .with_text("No terminals in this workspace".to_string()),
                )
                .with_child(button),
        )
}

/// Vertical divider between columns within a row. Dragging left/right adjusts
/// the flex-grow ratios of the two adjacent panes.
fn build_col_resizer(row_idx: usize, col_idx: usize, shared: &SharedState) -> ElementDef {
    let drag_shared = shared.clone();
    ElementDef::new(Tag::Div)
        .with_class("resizer")
        .with_class("resizer-h")
        .on_drag(move |ev| match ev.phase {
            DragPhase::Start => {
                mutate_with(&drag_shared, |st| {
                    st.resize_drag = Some(ResizeDragSnapshot {
                        horizontal: true,
                        row_idx,
                        col_idx: col_idx - 1,
                        initial_ratios: st.col_ratios[row_idx].clone(),
                        container_size: st.last_grid_width,
                    });
                });
            }
            DragPhase::Update => {
                mutate_with(&drag_shared, |st| {
                    let drag = match st.resize_drag {
                        Some(ref d) => d.clone(),
                        None => return,
                    };
                    apply_ratio_delta(
                        &mut st.col_ratios[drag.row_idx],
                        drag.col_idx,
                        drag.col_idx + 1,
                        &drag.initial_ratios,
                        ev.total_delta_x,
                        drag.container_size,
                    );
                });
            }
            DragPhase::End => {
                mutate_with(&drag_shared, |st| {
                    st.resize_drag = None;
                });
            }
        })
}

/// Horizontal divider between rows. Dragging up/down adjusts the flex-grow
/// ratios of the two adjacent rows.
fn build_row_resizer(row_idx: usize, shared: &SharedState) -> ElementDef {
    let drag_shared = shared.clone();
    ElementDef::new(Tag::Div)
        .with_class("resizer")
        .with_class("resizer-v")
        .on_drag(move |ev| match ev.phase {
            DragPhase::Start => {
                mutate_with(&drag_shared, |st| {
                    st.resize_drag = Some(ResizeDragSnapshot {
                        horizontal: false,
                        row_idx: row_idx - 1,
                        col_idx: 0,
                        initial_ratios: st.row_ratios.clone(),
                        container_size: st.last_grid_height,
                    });
                });
            }
            DragPhase::Update => {
                mutate_with(&drag_shared, |st| {
                    let drag = match st.resize_drag {
                        Some(ref d) => d.clone(),
                        None => return,
                    };
                    apply_ratio_delta(
                        &mut st.row_ratios,
                        drag.row_idx,
                        drag.row_idx + 1,
                        &drag.initial_ratios,
                        ev.total_delta_y,
                        drag.container_size,
                    );
                });
            }
            DragPhase::End => {
                mutate_with(&drag_shared, |st| {
                    st.resize_drag = None;
                });
            }
        })
}

fn build_pane(
    pane: &Pane,
    is_active: bool,
    capture_keyboard: bool,
    single_pane: bool,
    state: &UiSnapshot,
    shared: &SharedState,
    grids: &std::collections::HashMap<u32, unshit::core::cell_grid::CellGrid>,
) -> ElementDef {
    // Stable keys on both the pane container and its children keep the
    // reconciler from positionally shuffling DOM nodes when optional
    // children (header, drop-zone overlay) appear/disappear during
    // drags or split/unsplit operations.
    let mut container = ElementDef::new(Tag::Div)
        .with_class("pane")
        .with_key(format!("pane:{}", pane.id.0));
    if is_active {
        container = container.with_class("active");
    }
    let activate_state = shared.clone();
    let pane_id = pane.id;
    container = container.on_click(move || {
        mutate_with(&activate_state, |st| {
            let ws_idx = st.active_workspace;
            crate::state::dispatch(st, &format!("terminal.focus:{}:{}", ws_idx, pane_id.0));
        });
    });

    // Header is only meaningful in multi-pane tabs: the tab bar already
    // shows title/subtitle for the single-pane case, and the pane's drag
    // grip (used by the extract-to-tab flow) would have nothing to act on
    // when there is only one pane in the tab.
    if !single_pane {
        let header = build_pane_header(pane, shared).with_key("pane-header");
        container = container.with_child(header);
    }
    let body = build_pane_body(pane.id, capture_keyboard, state.font_size_pt, shared, grids)
        .with_key("pane-body");
    container = container.with_child(body);
    // During a tab drag, layer the 4-edge drop overlay inside the pane
    // so it tracks the pane's real layout rather than a recomputed
    // window rect that would drift under border/padding changes.
    if let Some(overlay) = crate::drag::overlay::build_pane_drop_zone_overlay(state, pane.id) {
        container = container.with_child(overlay.with_key("drop-overlay"));
    }
    container
}

fn build_pane_header(pane: &Pane, shared: &SharedState) -> ElementDef {
    let meta = format!("pid {} \u{00B7} {:.1}%", pane.pid, pane.cpu);
    let pane_id = pane.id;
    let split_h_state = shared.clone();
    let split_v_state = shared.clone();
    let close_state = shared.clone();
    let grip_state = shared.clone();
    let header = ElementDef::new(Tag::Div)
        .with_class("pane-header")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("pane-header-left")
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("pane-grip")
                        // Text-based 6-dot grip (two VERTICAL ELLIPSIS
                        // chars). Avoids an SVG allocation per pane
                        // which would blow the framework renderer's
                        // instance buffer past 4 panes.
                        .with_text("\u{22EE}\u{22EE}")
                        .on_drag(move |ev| match ev.phase {
                            DragPhase::Start => {
                                mutate_with(&grip_state, |st| {
                                    crate::state::dispatch(
                                        st,
                                        &format!("drag.start_pane:{}:{}:{}", pane_id.0, ev.x, ev.y),
                                    );
                                });
                            }
                            DragPhase::Update => {
                                mutate_with(&grip_state, |st| {
                                    crate::state::dispatch(
                                        st,
                                        &format!("drag.update:{}:{}", ev.x, ev.y),
                                    );
                                });
                            }
                            DragPhase::End => {
                                mutate_with(&grip_state, |st| {
                                    crate::state::dispatch(
                                        st,
                                        &format!("drag.update:{}:{}", ev.x, ev.y),
                                    );
                                    crate::state::dispatch(st, "drag.end");
                                });
                            }
                        }),
                )
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
    capture_keyboard: bool,
    font_size_pt: u32,
    shared: &SharedState,
    grids: &std::collections::HashMap<u32, unshit::core::cell_grid::CellGrid>,
) -> ElementDef {
    let mut body = ElementDef::new(Tag::Div).with_class("pane-body");

    if let Some(grid) = grids.get(&pane_id.0) {
        // Real terminal grid rendering.
        let mut grid_el = ElementDef::new(Tag::Div)
            .with_class("terminal-content")
            .with_style(StyleDeclaration::FontSize(font_size_pt as f32))
            .with_style(StyleDeclaration::LineHeight(terminal_line_height()))
            .with_grid(grid.clone())
            .with_persistent_buffer(true);
        let content_x_offset = terminal_content_x_offset();
        if content_x_offset.abs() > f32::EPSILON {
            grid_el = grid_el.with_style(StyleDeclaration::TransformTranslateX(TransformX::Px(
                content_x_offset,
            )));
        }

        // A tab_index is required so the element is focusable; without it the
        // framework ignores click-to-focus and keyboard events never arrive.
        grid_el = grid_el.with_tab_index(0);

        if capture_keyboard {
            grid_el = grid_el.captures_keyboard(true);

            // Register keyboard capture handler to send input to PTY.
            // Shift+PageUp/Down are intercepted for scrollback navigation
            // instead of being forwarded to the shell.
            let kbd_shared = shared.clone();
            let kbd_pane_id = pane_id;
            grid_el = grid_el.on(
                EventType::KeyboardCapture,
                move |event: &Event| -> Option<Box<dyn std::any::Any>> {
                    if let Event::Keyboard(kb) = event {
                        if kb.kind != KeyEventKind::Pressed {
                            return None;
                        }

                        let has_shift = kb.modifiers.contains(Modifiers::SHIFT);
                        let no_ctrl = !kb.modifiers.contains(Modifiers::CTRL);
                        let no_alt = !kb.modifiers.contains(Modifiers::ALT);

                        // Shift+PageUp: scroll back half a page.
                        if has_shift && no_ctrl && no_alt && kb.key == Key::PageUp {
                            mutate_with(&kbd_shared, |st| {
                                if let Some(handle) = st.terminals.get(&kbd_pane_id.0) {
                                    let mut terminal = handle.lock_recover();
                                    let half = (terminal.grid().rows() / 2).max(1);
                                    terminal.scroll_view_up(half);
                                }
                            });
                            return None;
                        }

                        // Shift+PageDown: scroll forward half a page.
                        if has_shift && no_ctrl && no_alt && kb.key == Key::PageDown {
                            mutate_with(&kbd_shared, |st| {
                                if let Some(handle) = st.terminals.get(&kbd_pane_id.0) {
                                    let mut terminal = handle.lock_recover();
                                    let half = (terminal.grid().rows() / 2).max(1);
                                    terminal.scroll_view_down(half);
                                }
                            });
                            return None;
                        }

                        // Any other key while scrolled back snaps to live view.
                        mutate_with(&kbd_shared, |st| {
                            if let Some(handle) = st.terminals.get(&kbd_pane_id.0) {
                                let mut terminal = handle.lock_recover();
                                if terminal.scroll_offset() > 0 {
                                    terminal.reset_scroll();
                                }
                            }
                        });

                        if let Some(bytes) = crate::terminal::keys::encode_key(kb) {
                            mutate_with(&kbd_shared, |st| {
                                let _ = st.pty_manager.write(kbd_pane_id.0, &bytes);
                            });
                        }
                    }
                    None
                },
            );

            // Mouse wheel scrolls the scrollback buffer.
            // delta_y > 0 = wheel up (toward older history).
            // delta_y < 0 = wheel down (toward live screen).
            // The framework converts LineDelta to pixels (line_height ~40px),
            // so divide by cell_h to get terminal lines. Minimum 1 line per
            // notch so small deltas still move.
            let scroll_shared = shared.clone();
            let scroll_pane_id = pane_id;
            grid_el = grid_el.on(
                EventType::Scroll,
                move |event: &Event| -> Option<Box<dyn std::any::Any>> {
                    if let Event::Scroll(se) = event {
                        use unshit::core::cell_grid::CellGrid;
                        let cell_h = CellGrid::global_cell_h().max(1.0);
                        let raw_lines = se.delta_y / cell_h;
                        // Round away from zero so even a small notch scrolls 1 line.
                        let lines = if raw_lines > 0.0 {
                            raw_lines.ceil() as i32
                        } else {
                            raw_lines.floor() as i32
                        };
                        if lines != 0 {
                            mutate_with(&scroll_shared, |st| {
                                if let Some(handle) = st.terminals.get(&scroll_pane_id.0) {
                                    let mut terminal = handle.lock_recover();
                                    if lines > 0 {
                                        terminal.scroll_view_up(lines as usize);
                                    } else {
                                        terminal.scroll_view_down((-lines) as usize);
                                    }
                                }
                            });
                        }
                    }
                    None
                },
            );

            // Register resize handler to update PTY dimensions.
            // Prefer the renderer-computed pending resize (exact), fall
            // back to global cell metrics, then to hardcoded estimates.
            // Note: this fires with THIS pane's terminal-content size, not
            // the whole grid. The full-grid dimensions used for hit-testing
            // are captured by `.terminal-grid`'s own on_resize (see
            // `build_terminal_grid`).
            let resize_shared = shared.clone();
            let resize_pane_id = pane_id;
            grid_el = grid_el.on_resize(move |w, h| {
                use unshit::core::cell_grid::CellGrid;

                mutate_with(&resize_shared, |st| {
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
                        if let Some(handle) = st.terminals.get(&resize_pane_id.0) {
                            handle
                                .lock()
                                .expect("terminal mutex poisoned")
                                .resize(rows as usize, cols as usize);
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
    use crate::state::{seed_state, Pane, PaneId};
    use std::sync::{Arc, Mutex};
    use unshit::core::cell_grid::CellGrid;

    fn make_shared() -> SharedState {
        Arc::new(Mutex::new(seed_state()))
    }

    fn make_snapshot() -> UiSnapshot {
        seed_state().ui_snapshot()
    }

    fn make_pane_titled(id: u32, title: &str) -> Pane {
        Pane {
            id: PaneId(id),
            title: title.to_string(),
            subtitle: "bash".to_string(),
            pid: 1234,
            cpu: 5.3,
        }
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

    /// Build a minimal shared state for testing. Does not spawn any real PTY.
    fn test_shared() -> SharedState {
        Arc::new(Mutex::new(crate::state::seed_state()))
    }

    fn find_by_class<'a>(def: &'a ElementDef, class: &str) -> Option<&'a ElementDef> {
        if def.classes.iter().any(|c| c == class) {
            return Some(def);
        }
        for child in &def.children {
            if let Some(found) = find_by_class(child, class) {
                return Some(found);
            }
        }
        None
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

    // -- build_terminal_grid: empty workspace -----------------------------------

    #[test]
    fn terminal_grid_empty_panes_shows_new_terminal_cta() {
        let mut state = seed_state();
        state.panes = vec![];
        state.tabs = vec![];
        let snap = state.ui_snapshot();
        let shared = make_shared();
        let grids = std::collections::HashMap::new();
        let el = build_terminal_grid(&snap, &shared, &grids);
        assert!(el.classes.contains(&"empty".to_string()));
        let btn = find_by_class(&el, "empty-workspace-new-terminal")
            .expect("new terminal button missing");
        assert!(btn.on_click.is_some());
    }

    #[test]
    fn terminal_grid_empty_button_dispatches_tab_new() {
        let shared = make_shared();
        {
            let mut guard = shared.lock().unwrap();
            guard.panes = vec![];
            guard.tabs = vec![];
        }
        let snap = shared.lock().unwrap().ui_snapshot();
        let grids = std::collections::HashMap::new();
        let el = build_terminal_grid(&snap, &shared, &grids);
        let btn = find_by_class(&el, "empty-workspace-new-terminal").unwrap();
        (btn.on_click.as_ref().unwrap())();
        let guard = shared.lock().unwrap();
        assert!(!guard.tabs.is_empty(), "tab.new must populate state.tabs");
        assert!(!guard.panes.is_empty(), "tab.new must populate state.panes");
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
            vec![make_pane_titled(1, "a"), make_pane_titled(2, "b")],
            vec![make_pane_titled(3, "c"), make_pane_titled(4, "d")],
        ];
        state.active_pane = PaneId(1);
        let snap = state.ui_snapshot();
        let shared = make_shared();
        let grids = std::collections::HashMap::new();
        let el = build_terminal_grid(&snap, &shared, &grids);

        // 2 rows + 1 vertical resizer between them = 3 children
        assert_eq!(el.children.len(), 3);

        // The vertical resizer is at index 1 (now uses resizer/resizer-v classes)
        let v_resizer = &el.children[1];
        assert!(v_resizer.classes.contains(&"resizer".to_string()));
        assert!(v_resizer.classes.contains(&"resizer-v".to_string()));

        // Each row has 2 panes + 1 horizontal resizer = 3 children
        let row0 = &el.children[0];
        assert_eq!(row0.children.len(), 3);
        let h_resizer = &row0.children[1];
        assert!(h_resizer.classes.contains(&"resizer".to_string()));
        assert!(h_resizer.classes.contains(&"resizer-h".to_string()));

        let row1 = &el.children[2];
        assert_eq!(row1.children.len(), 3);
    }

    // -- build_pane: active vs inactive -----------------------------------------

    #[test]
    fn pane_active_has_active_class() {
        let pane = make_pane_titled(1, "shell");
        let shared = make_shared();
        let grids = std::collections::HashMap::new();
        let el = build_pane(&pane, true, true, false, &make_snapshot(), &shared, &grids);
        assert!(el.classes.contains(&"pane".to_string()));
        assert!(el.classes.contains(&"active".to_string()));
    }

    #[test]
    fn pane_inactive_lacks_active_class() {
        let pane = make_pane_titled(1, "shell");
        let shared = make_shared();
        let grids = std::collections::HashMap::new();
        let el = build_pane(
            &pane,
            false,
            false,
            false,
            &make_snapshot(),
            &shared,
            &grids,
        );
        assert!(el.classes.contains(&"pane".to_string()));
        assert!(!el.classes.contains(&"active".to_string()));
    }

    #[test]
    fn pane_has_header_and_body() {
        let pane = make_pane_titled(1, "shell");
        let shared = make_shared();
        let grids = std::collections::HashMap::new();
        let el = build_pane(&pane, true, true, false, &make_snapshot(), &shared, &grids);
        assert_eq!(el.children.len(), 2);
        assert!(el.children[0].classes.contains(&"pane-header".to_string()));
        assert!(el.children[1].classes.contains(&"pane-body".to_string()));
    }

    #[test]
    fn pane_has_click_handler() {
        let pane = make_pane_titled(1, "shell");
        let shared = make_shared();
        let grids = std::collections::HashMap::new();
        let el = build_pane(
            &pane,
            false,
            false,
            false,
            &make_snapshot(),
            &shared,
            &grids,
        );
        assert!(el.on_click.is_some());
    }

    // -- build_pane_header ------------------------------------------------------

    #[test]
    fn pane_header_has_correct_class() {
        let pane = make_pane_titled(42, "zsh");
        let shared = make_shared();
        let el = build_pane_header(&pane, &shared);
        assert!(el.classes.contains(&"pane-header".to_string()));
    }

    #[test]
    fn pane_header_has_three_sections() {
        let pane = make_pane_titled(42, "zsh");
        let shared = make_shared();
        let el = build_pane_header(&pane, &shared);
        // left, meta, right
        assert_eq!(el.children.len(), 3);
        assert!(el.children[0]
            .classes
            .contains(&"pane-header-left".to_string()));
        assert!(el.children[1].classes.contains(&"pane-meta".to_string()));
        assert!(el.children[2]
            .classes
            .contains(&"pane-header-right".to_string()));
    }

    #[test]
    fn pane_header_meta_shows_pid_and_cpu() {
        let pane = make_pane_titled(42, "zsh");
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
        let pane = make_pane_titled(1, "shell");
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
        let el = build_pane_body(PaneId(1), true, 13, &shared, &grids);
        assert!(el.classes.contains(&"pane-body".to_string()));
        assert_eq!(el.children.len(), 1);
        let grid_el = &el.children[0];
        assert!(grid_el.classes.contains(&"terminal-content".to_string()));
        assert!(grid_el.persistent_buffer);
    }

    #[test]
    fn pane_body_applies_configured_terminal_font_metrics() {
        let shared = make_shared();
        let mut grids = std::collections::HashMap::new();
        grids.insert(1, CellGrid::new(24, 80));

        let el = build_pane_body(PaneId(1), true, 18, &shared, &grids);
        let grid_el = &el.children[0];

        assert!(
            grid_el.style_overrides.iter().any(|decl| {
                matches!(
                    decl,
                    StyleDeclaration::FontSize(v) if (*v - 18.0).abs() < f32::EPSILON
                )
            }),
            "terminal-content must use the configured terminal font size"
        );
        assert!(
            grid_el.style_overrides.iter().any(|decl| {
                matches!(
                    decl,
                    StyleDeclaration::LineHeight(v)
                        if (*v - CSS_LINE_HEIGHT).abs() < f32::EPSILON
                )
            }),
            "terminal-content line-height must stay in sync with cell metrics"
        );
    }

    #[test]
    fn terminal_line_height_from_values_uses_windows_terminal_profile_default() {
        let got = terminal_line_height_from_values(None, true);
        assert!((got - WINDOWS_TERMINAL_PARITY_LINE_HEIGHT).abs() < f32::EPSILON);
    }

    #[test]
    fn terminal_line_height_from_values_accepts_tuning_override() {
        let got = terminal_line_height_from_values(Some(std::ffi::OsString::from("1.12")), true);
        assert!((got - 1.12).abs() < f32::EPSILON);
    }

    #[test]
    fn terminal_line_height_from_values_rejects_invalid_tuning_override() {
        for value in ["invalid", "0.25", "2.5"] {
            let got = terminal_line_height_from_values(Some(std::ffi::OsString::from(value)), true);
            assert!(
                (got - WINDOWS_TERMINAL_PARITY_LINE_HEIGHT).abs() < f32::EPSILON,
                "invalid override {value:?} should fall back to the parity default"
            );
        }
    }

    #[test]
    fn terminal_content_x_offset_from_values_uses_windows_terminal_profile_default() {
        let got = terminal_content_x_offset_from_values(None, true);
        assert!((got - WINDOWS_TERMINAL_PARITY_CONTENT_X_OFFSET).abs() < f32::EPSILON);
    }

    #[test]
    fn terminal_content_x_offset_from_values_accepts_tuning_override() {
        let got =
            terminal_content_x_offset_from_values(Some(std::ffi::OsString::from("6.5")), true);
        assert!((got - 6.5).abs() < f32::EPSILON);
    }

    #[test]
    fn terminal_content_x_offset_from_values_rejects_invalid_tuning_override() {
        for value in ["invalid", "-33", "33"] {
            let got =
                terminal_content_x_offset_from_values(Some(std::ffi::OsString::from(value)), true);
            assert!(
                (got - WINDOWS_TERMINAL_PARITY_CONTENT_X_OFFSET).abs() < f32::EPSILON,
                "invalid override {value:?} should fall back to the parity default"
            );
        }
    }

    #[test]
    fn terminal_content_x_offset_from_values_stays_zero_outside_parity_profile() {
        let got = terminal_content_x_offset_from_values(None, false);
        assert!(got.abs() < f32::EPSILON);
    }

    #[test]
    fn parity_windows_terminal_profile_from_value_rejects_disabled_values() {
        for value in [
            None,
            Some(""),
            Some("0"),
            Some("false"),
            Some("off"),
            Some("no"),
        ] {
            let value = value.map(std::ffi::OsString::from);
            assert!(!parity_windows_terminal_profile_from_value(value));
        }
    }

    #[test]
    fn parity_windows_terminal_profile_from_value_accepts_enabled_values() {
        for value in ["1", "true", "yes", "on"] {
            assert!(parity_windows_terminal_profile_from_value(Some(
                std::ffi::OsString::from(value)
            )));
        }
    }

    #[test]
    fn pane_body_with_grid_active_captures_keyboard() {
        let shared = make_shared();
        let mut grids = std::collections::HashMap::new();
        grids.insert(1, CellGrid::new(24, 80));
        let el = build_pane_body(PaneId(1), true, 13, &shared, &grids);
        let grid_el = &el.children[0];
        assert!(grid_el.captures_keyboard);
    }

    #[test]
    fn pane_body_with_grid_inactive_does_not_capture_keyboard() {
        let shared = make_shared();
        let mut grids = std::collections::HashMap::new();
        grids.insert(1, CellGrid::new(24, 80));
        let el = build_pane_body(PaneId(1), false, 13, &shared, &grids);
        let grid_el = &el.children[0];
        assert!(!grid_el.captures_keyboard);
    }

    // -- build_pane_body: without grid (fallback) -------------------------------

    #[test]
    fn pane_body_without_grid_shows_fallback() {
        let shared = make_shared();
        let grids = std::collections::HashMap::new(); // no grid for pane 1
        let el = build_pane_body(PaneId(1), true, 13, &shared, &grids);
        assert!(el.classes.contains(&"pane-body".to_string()));
        assert_eq!(el.children.len(), 1);
        let fallback = &el.children[0];
        assert!(fallback.classes.contains(&"term-line".to_string()));
        // Should have prompt and cursor children
        assert_eq!(fallback.children.len(), 2);
        assert!(fallback.children[0]
            .classes
            .contains(&"term-prompt".to_string()));
        assert!(fallback.children[1]
            .classes
            .contains(&"term-cursor".to_string()));
    }

    #[test]
    fn pane_body_without_grid_inactive_also_shows_fallback() {
        let shared = make_shared();
        let grids = std::collections::HashMap::new();
        let el = build_pane_body(PaneId(99), false, 13, &shared, &grids);
        assert_eq!(el.children.len(), 1);
        assert!(el.children[0].classes.contains(&"term-line".to_string()));
    }

    // -- closure invocation tests (cover on_click/on_resize bodies) ------------

    #[test]
    fn pane_click_sets_active_pane() {
        let shared = make_shared();
        let pane = make_pane_titled(42, "shell");
        {
            let mut guard = shared.lock().unwrap();
            guard.panes[0].push(pane.clone());
            guard.col_ratios[0].push(1.0);
        }
        let grids = std::collections::HashMap::new();
        let el = build_pane(
            &pane,
            false,
            false,
            false,
            &make_snapshot(),
            &shared,
            &grids,
        );
        (el.on_click.as_ref().unwrap())();
        assert_eq!(shared.lock().unwrap().active_pane, PaneId(42));
    }

    #[test]
    fn pane_header_split_h_has_click_handler() {
        let shared = make_shared();
        let pane = make_pane_titled(1, "shell");
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
        let pane = make_pane_titled(1, "shell");
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
        let pane = make_pane_titled(1, "shell");
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
        let el = build_pane_body(PaneId(1), true, 13, &shared, &grids);
        let grid_el = &el.children[0];
        // Should have event handlers registered (KeyboardCapture)
        assert!(!grid_el.handlers.is_empty());
    }

    #[test]
    fn pane_body_active_grid_has_resize_handler() {
        let shared = make_shared();
        let mut grids = std::collections::HashMap::new();
        grids.insert(1, CellGrid::new(24, 80));
        let el = build_pane_body(PaneId(1), true, 13, &shared, &grids);
        let grid_el = &el.children[0];
        assert!(grid_el.on_resize.is_some());
    }

    #[test]
    fn pane_body_inactive_grid_has_no_keyboard_handler() {
        let shared = make_shared();
        let mut grids = std::collections::HashMap::new();
        grids.insert(1, CellGrid::new(24, 80));
        let el = build_pane_body(PaneId(1), false, 13, &shared, &grids);
        let grid_el = &el.children[0];
        assert!(grid_el.handlers.is_empty());
        assert!(grid_el.on_resize.is_none());
    }

    #[test]
    fn pane_body_resize_handler_invocation() {
        let shared = make_shared();
        let mut grids = std::collections::HashMap::new();
        grids.insert(1, CellGrid::new(24, 80));
        let el = build_pane_body(PaneId(1), true, 13, &shared, &grids);
        let grid_el = &el.children[0];
        let resize_fn = grid_el.on_resize.as_ref().unwrap();
        // Invoke with a 640x384 area (should yield 80 cols, 24 rows)
        (resize_fn)(640.0, 384.0);
        // The resize handler should not panic and should work
    }

    #[test]
    fn pane_header_left_has_grip_dot_title_subtitle() {
        let pane = make_pane_titled(1, "zsh");
        let shared = make_shared();
        let el = build_pane_header(&pane, &shared);
        let left = &el.children[0];
        assert!(left.classes.contains(&"pane-header-left".to_string()));
        assert_eq!(left.children.len(), 4);
        assert!(left.children[0].classes.contains(&"pane-grip".to_string()));
        assert!(left.children[1]
            .classes
            .contains(&"pane-status-dot".to_string()));
        assert!(left.children[2].classes.contains(&"pane-title".to_string()));
        assert!(left.children[3]
            .classes
            .contains(&"pane-subtitle".to_string()));
    }

    #[test]
    fn terminal_grid_with_three_cols() {
        let mut state = seed_state();
        state.panes = vec![vec![
            make_pane_titled(1, "a"),
            make_pane_titled(2, "b"),
            make_pane_titled(3, "c"),
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

    // -- base branch tests (regression, pane-header deduplication) -------------

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

        let body = build_pane_body(pane_id, true, 13, &shared, &grids);
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
    fn active_pane_captures_keyboard_base() {
        let shared = test_shared();
        let pane_id = PaneId(1);
        let grid = CellGrid::new(24, 80);
        let mut grids = std::collections::HashMap::new();
        grids.insert(pane_id.0, grid);

        let body = build_pane_body(pane_id, true, 13, &shared, &grids);
        let content = find_terminal_content(&body).expect("terminal-content element should exist");
        assert!(
            content.captures_keyboard,
            "active pane terminal-content must capture keyboard"
        );
    }

    /// An inactive pane should still be focusable (tab_index set) but must
    /// NOT capture the keyboard so that shortcuts keep working.
    #[test]
    fn inactive_pane_does_not_capture_keyboard_base() {
        let shared = test_shared();
        let pane_id = PaneId(1);
        let grid = CellGrid::new(24, 80);
        let mut grids = std::collections::HashMap::new();
        grids.insert(pane_id.0, grid);

        let body = build_pane_body(pane_id, false, 13, &shared, &grids);
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

    /// When the settings modal is open, the active pane must NOT capture
    /// the keyboard so that Escape and other modal shortcuts reach the
    /// dialog instead of being forwarded to the PTY.
    #[test]
    fn active_pane_does_not_capture_keyboard_when_settings_open() {
        let mut state = seed_state();
        state.settings_open = true;
        let pane = state.panes[0][0].clone();
        state.active_pane = pane.id;
        let snap = state.ui_snapshot();
        let shared = make_shared();
        let mut grids = std::collections::HashMap::new();
        grids.insert(pane.id.0, CellGrid::new(24, 80));

        let el = build_terminal_grid(&snap, &shared, &grids);
        let content = find_terminal_content(&el)
            .expect("terminal-content element should exist when grid is present");
        assert!(
            !content.captures_keyboard,
            "active pane must not capture keyboard while settings modal is open"
        );
    }

    /// Active pane must register an on_resize handler so the PTY dimensions
    /// stay in sync with the visible grid area.
    #[test]
    fn active_pane_registers_resize_handler_base() {
        let shared = test_shared();
        let pane_id = PaneId(1);
        let grid = CellGrid::new(24, 80);
        let mut grids = std::collections::HashMap::new();
        grids.insert(pane_id.0, grid);

        let body = build_pane_body(pane_id, true, 13, &shared, &grids);
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
    // Pane header visibility: single-pane tabs omit the header entirely
    // -----------------------------------------------------------------------

    /// Single-pane tabs have no pane header at all. The tab bar already
    /// shows title/subtitle and a single pane cannot be extracted, so
    /// the grip and action buttons would be clutter.
    #[test]
    fn single_pane_tab_omits_pane_header() {
        let shared = test_shared();
        let pane = make_pane(1);
        let grids = std::collections::HashMap::new();
        let el = build_pane(&pane, true, true, true, &make_snapshot(), &shared, &grids);

        assert!(
            !tree_has_class(&el, "pane-header"),
            "single-pane tab must not render a pane-header"
        );
    }

    /// Multi-pane tabs include a header with a grip on the left so each
    /// pane can be dragged. The header also carries title/subtitle and
    /// action buttons so the user can distinguish panes and manage them.
    #[test]
    fn multi_pane_tab_renders_header_with_grip() {
        let shared = test_shared();
        let pane = make_pane(1);
        let grids = std::collections::HashMap::new();
        let el = build_pane(&pane, true, true, false, &make_snapshot(), &shared, &grids);

        assert!(
            tree_has_class(&el, "pane-header"),
            "multi-pane tab must render a pane-header"
        );
        assert!(
            tree_has_class(&el, "pane-grip"),
            "multi-pane header must include a pane-grip for drag"
        );
        assert!(
            tree_has_class(&el, "pane-title"),
            "multi-pane header must show the pane title"
        );
        assert!(
            tree_has_text(&el, "shell"),
            "multi-pane header must display the pane title text"
        );
    }

    /// The grip on the pane header is the drag source for the
    /// pane-extract-to-tab flow (F4). It must carry an `on_drag`
    /// handler so the framework tracks the pointer.
    #[test]
    fn pane_grip_has_on_drag_handler() {
        let shared = test_shared();
        let pane = make_pane(1);
        let header = build_pane_header(&pane, &shared);
        let left = &header.children[0];
        let grip = &left.children[0];
        assert!(grip.classes.contains(&"pane-grip".to_string()));
        assert!(
            grip.on_drag.is_some(),
            "pane grip must have on_drag handler for extract-to-tab"
        );
    }

    /// Invoking the grip's `on_drag` with a `Start` phase must transition
    /// the app's drag state to `DraggingPane` for the owning pane id.
    #[test]
    fn pane_grip_on_drag_start_enters_dragging_state() {
        use unshit::core::event::MouseButton;
        use unshit::core::event::{DragEvent, DragPhase};
        let shared = test_shared();
        let pane = make_pane(42);
        {
            let mut guard = shared.lock().unwrap();
            guard.panes = vec![vec![pane.clone()]];
            guard.active_pane = pane.id;
        }
        let header = build_pane_header(&pane, &shared);
        let grip = &header.children[0].children[0];
        let handler = grip.on_drag.as_ref().unwrap().clone();
        let start = DragEvent {
            phase: DragPhase::Start,
            x: 100.0,
            y: 200.0,
            delta_x: 0.0,
            delta_y: 0.0,
            total_delta_x: 0.0,
            total_delta_y: 0.0,
            button: MouseButton::Left,
        };
        handler(&start);

        let guard = shared.lock().unwrap();
        match &guard.drag {
            crate::drag::DragState::DraggingPane { pane: p, .. } => {
                assert_eq!(*p, PaneId(42));
            }
            _ => panic!("expected DraggingPane state after DragPhase::Start"),
        }
    }

    /// A `DragPhase::Update` event must refresh `cursor_x`/`cursor_y` on
    /// the active drag so the extract-to-tab end handler can hit-test.
    #[test]
    fn pane_grip_on_drag_update_refreshes_cursor() {
        use unshit::core::event::{DragEvent, DragPhase, MouseButton};
        let shared = test_shared();
        let pane = make_pane(7);
        {
            let mut guard = shared.lock().unwrap();
            guard.panes = vec![vec![pane.clone()]];
            guard.active_pane = pane.id;
        }
        let header = build_pane_header(&pane, &shared);
        let grip = &header.children[0].children[0];
        let handler = grip.on_drag.as_ref().unwrap().clone();
        let start = DragEvent {
            phase: DragPhase::Start,
            x: 0.0,
            y: 0.0,
            delta_x: 0.0,
            delta_y: 0.0,
            total_delta_x: 0.0,
            total_delta_y: 0.0,
            button: MouseButton::Left,
        };
        handler(&start);
        let update = DragEvent {
            phase: DragPhase::Update,
            x: 321.0,
            y: 54.0,
            delta_x: 5.0,
            delta_y: 2.0,
            total_delta_x: 321.0,
            total_delta_y: 54.0,
            button: MouseButton::Left,
        };
        handler(&update);

        assert_eq!(
            shared.lock().unwrap().drag.cursor(),
            Some((321.0, 54.0)),
            "drag cursor must reflect the latest update event"
        );
    }

    /// `DragPhase::End` over the tab bar must extract the pane into a
    /// new tab. This exercises the full callback path from framework
    /// event to dispatch to mutate_extract_pane_to_tab.
    #[test]
    fn pane_grip_on_drag_end_over_tabbar_extracts() {
        use crate::state::mutate_split_right;
        use unshit::core::event::{DragEvent, DragPhase, MouseButton};
        let shared = test_shared();
        let original;
        let extracted;
        {
            let mut guard = shared.lock().unwrap();
            original = guard.active_pane;
            mutate_split_right(&mut guard, original);
            extracted = guard.active_pane;
            guard.tabbar_rect = crate::drag::Rect {
                x: 0.0,
                y: 34.0,
                width: 800.0,
                height: 38.0,
            };
        }
        let tabs_before = shared.lock().unwrap().tabs.len();

        let pane = Pane {
            id: extracted,
            title: "shell".into(),
            subtitle: "bash".into(),
            pid: 0,
            cpu: 0.0,
        };
        let header = build_pane_header(&pane, &shared);
        let grip = &header.children[0].children[0];
        let handler = grip.on_drag.as_ref().unwrap().clone();

        let mk = |phase, x, y| DragEvent {
            phase,
            x,
            y,
            delta_x: 0.0,
            delta_y: 0.0,
            total_delta_x: 0.0,
            total_delta_y: 0.0,
            button: MouseButton::Left,
        };
        handler(&mk(DragPhase::Start, 400.0, 300.0));
        handler(&mk(DragPhase::Update, 600.0, 50.0));
        handler(&mk(DragPhase::End, 600.0, 50.0));

        assert_eq!(
            shared.lock().unwrap().tabs.len(),
            tabs_before + 1,
            "dropping on the tab bar must spawn a new tab"
        );
        assert!(
            matches!(shared.lock().unwrap().drag, crate::drag::DragState::Idle),
            "drag state must reset after end"
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
