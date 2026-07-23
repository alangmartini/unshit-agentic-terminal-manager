//! Pane body for built-in file editor panes.
//!
//! Mirrors the terminal pane's grid element (`terminal_grid.rs`): the
//! editor's live `CellGrid` is attached with a persistent buffer, the
//! element captures keyboard input when the pane is active, and wheel
//! scrolls return paint-only grid patches instead of tree rebuilds.

use unshit::core::element::*;
use unshit::core::event::{Event, EventType, Key, KeyEventKind, Modifiers};
use unshit::core::style::parse::StyleDeclaration;

use crate::state::{mutate_with, PaneId, SharedState};

/// Lines per PageUp/PageDown step for a viewport of `rows`.
fn page_step(rows: usize) -> isize {
    rows.saturating_sub(1).max(1) as isize
}

/// Build the editor pane body. `capture_keyboard` is true for the active
/// pane only, exactly like terminal panes.
pub fn build_editor_pane_body(
    pane_id: PaneId,
    capture_keyboard: bool,
    font_size_pt: u32,
    shared: &SharedState,
    grids: &std::collections::HashMap<u32, unshit::core::cell_grid::CellGrid>,
) -> ElementDef {
    let mut body = ElementDef::new(Tag::Div).with_class("pane-body");

    let Some(grid) = grids.get(&pane_id.0) else {
        return body;
    };

    let mut grid_el = ElementDef::new(Tag::Div)
        .with_class("terminal-content")
        .with_class("editor-content")
        .with_style(StyleDeclaration::FontSize(font_size_pt as f32))
        .with_style(StyleDeclaration::LineHeight(
            crate::ui::terminal_grid::terminal_line_height(),
        ))
        .with_style(StyleDeclaration::FontScale(1.0))
        .with_grid(grid.clone())
        .with_persistent_buffer(true)
        // Focusability requires a tab index (see terminal_grid.rs).
        .with_tab_index(0);

    if capture_keyboard {
        grid_el = grid_el.captures_keyboard(true);

        let kbd_shared = shared.clone();
        let kbd_pane = pane_id;
        grid_el = grid_el.on(
            EventType::KeyboardCapture,
            move |event: &Event| -> Option<Box<dyn std::any::Any>> {
                let Event::Keyboard(kb) = event else {
                    return None;
                };
                if kb.kind != KeyEventKind::Pressed {
                    return None;
                }
                let ctrl = kb.modifiers.contains(Modifiers::CTRL);
                let scrolled = mutate_with(&kbd_shared, |st| {
                    let editor = st.editors.get_mut(&kbd_pane.0)?;
                    let rows = editor.grid.rows();
                    let moved = match kb.key {
                        Key::ArrowUp => editor.scroll_by(-1),
                        Key::ArrowDown => editor.scroll_by(1),
                        Key::PageUp => editor.scroll_by(-page_step(rows)),
                        Key::PageDown => editor.scroll_by(page_step(rows)),
                        Key::Home if ctrl => editor.scroll_to(0),
                        Key::End if ctrl => {
                            let target = editor.max_top_line();
                            editor.scroll_to(target)
                        }
                        _ => return None,
                    };
                    Some(moved)
                });
                match scrolled {
                    Some(true) => Some(Box::new(unshit::core::event::RequestRebuild)),
                    _ => None,
                }
            },
        );

        // Wheel scroll: whole-line steps as a paint-only grid patch, no
        // tree rebuild (mirrors the terminal's instant wheel path).
        let scroll_shared = shared.clone();
        let scroll_pane = pane_id;
        grid_el = grid_el.on(
            EventType::Scroll,
            move |event: &Event| -> Option<Box<dyn std::any::Any>> {
                let Event::Scroll(se) = event else {
                    return None;
                };
                let cell_h = unshit::core::cell_grid::CellGrid::global_cell_h().max(1.0);
                // delta_y > 0 is wheel up (toward the top of the file).
                let lines = (se.delta_y / cell_h).round() as isize;
                let grid = mutate_with(&scroll_shared, |st| {
                    let editor = st.editors.get_mut(&scroll_pane.0)?;
                    let step = if lines == 0 {
                        if se.delta_y > 0.0 {
                            -1
                        } else if se.delta_y < 0.0 {
                            1
                        } else {
                            return None;
                        }
                    } else {
                        -lines
                    };
                    editor.scroll_by(step).then(|| editor.grid.clone())
                });
                Some(Box::new(unshit::app::app::ScrollGridPatch {
                    grid,
                    animation: None,
                }))
            },
        );
    }

    // Track the pane's rendered size so the viewport grid matches the
    // cell capacity of the element. No PTY is involved for editors.
    let resize_shared = shared.clone();
    let resize_pane = pane_id;
    grid_el = grid_el.on_resize(move |w, h| {
        use unshit::core::cell_grid::CellGrid;
        let cell_w = CellGrid::global_cell_w();
        let cell_h = CellGrid::global_cell_h();
        if cell_w <= 0.0 || cell_h <= 0.0 {
            return;
        }
        let cols = (w / cell_w).max(1.0) as usize;
        let rows = (h / cell_h).max(1.0) as usize;
        mutate_with(&resize_shared, |st| {
            if let Some(editor) = st.editors.get_mut(&resize_pane.0) {
                editor.resize(rows, cols);
            }
        });
    });

    body = body.with_child(grid_el);
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::seed_state;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    fn shared_with_editor() -> (SharedState, std::path::PathBuf) {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tm-editor-pane-ui-{}-{}.txt",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let text: Vec<String> = (0..30).map(|i| format!("row {}", i)).collect();
        std::fs::write(&path, text.join("\n")).expect("write temp file");
        let mut state = seed_state();
        let editor = crate::editor::EditorPane::open(&path, 10, 40).expect("open editor");
        state.editors.insert(1, editor);
        (Arc::new(Mutex::new(state)), path)
    }

    fn grids_for(
        shared: &SharedState,
    ) -> std::collections::HashMap<u32, unshit::core::cell_grid::CellGrid> {
        let guard = shared.lock().expect("state lock");
        guard
            .editors
            .iter()
            .map(|(&id, e)| (id, e.grid.clone()))
            .collect()
    }

    #[test]
    fn editor_pane_body_renders_grid_with_content() {
        let (shared, path) = shared_with_editor();
        let grids = grids_for(&shared);
        let el = build_editor_pane_body(PaneId(1), true, 13, &shared, &grids);
        assert!(el.classes.contains(&"pane-body".to_string()));
        assert_eq!(el.children.len(), 1);
        let grid_el = &el.children[0];
        assert!(grid_el.classes.contains(&"editor-content".to_string()));
        assert!(grid_el.classes.contains(&"terminal-content".to_string()));
        assert!(grid_el.persistent_buffer);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn active_editor_pane_captures_keyboard() {
        let (shared, path) = shared_with_editor();
        let grids = grids_for(&shared);
        let el = build_editor_pane_body(PaneId(1), true, 13, &shared, &grids);
        assert!(el.children[0].captures_keyboard);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn inactive_editor_pane_does_not_capture_keyboard() {
        let (shared, path) = shared_with_editor();
        let grids = grids_for(&shared);
        let el = build_editor_pane_body(PaneId(1), false, 13, &shared, &grids);
        assert!(!el.children[0].captures_keyboard);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn missing_grid_renders_empty_body() {
        let (shared, path) = shared_with_editor();
        let grids = std::collections::HashMap::new();
        let el = build_editor_pane_body(PaneId(1), true, 13, &shared, &grids);
        assert!(el.children.is_empty());
        let _ = std::fs::remove_file(path);
    }
}
