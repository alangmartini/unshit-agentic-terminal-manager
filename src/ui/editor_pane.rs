//! Pane body for built-in file editor panes.
//!
//! Mirrors the terminal pane's grid element (`terminal_grid.rs`): the
//! editor's live `CellGrid` is attached with a persistent buffer, the
//! element captures keyboard input when the pane is active, and wheel
//! scrolls return paint-only grid patches instead of tree rebuilds.

use unshit::core::element::*;
use unshit::core::event::{
    DragPhase, Event, EventType, Key, KeyEventKind, Modifiers, MouseButton, MouseEventKind,
};
use unshit::core::style::parse::StyleDeclaration;

use crate::state::{mutate_with, PaneId, SharedState};

/// Lines per PageUp/PageDown step for a viewport of `rows`.
fn page_step(rows: usize) -> isize {
    rows.saturating_sub(1).max(1) as isize
}

/// Printable text for an insert keystroke. Prefers the event's `text`
/// (dead-key/IME composition — see `terminal/keys.rs`), falling back to
/// the key's own character. Control characters never insert.
fn insert_text_for(kb: &unshit::core::event::KeyboardEvent) -> Option<String> {
    if let Some(text) = kb.text.as_ref() {
        let printable: String = text.chars().filter(|c| !c.is_control()).collect();
        if !printable.is_empty() {
            return Some(printable);
        }
    }
    match kb.key {
        Key::Char(c) if !c.is_control() => Some(c.to_string()),
        Key::Space => Some(" ".to_string()),
        _ => None,
    }
}

/// Translate one captured keystroke into editor buffer operations.
/// Returns `None` when the key is not an editor key (unconsumed),
/// `Some(changed)` otherwise. Global app shortcuts never reach this
/// handler — the framework resolves registered keybinds first.
pub(crate) fn handle_editor_key(
    st: &mut crate::state::AppState,
    pane_id: u32,
    kb: &unshit::core::event::KeyboardEvent,
) -> Option<bool> {
    use crate::editor::{Damage, TAB_SPACES};

    let ctrl = kb.modifiers.contains(Modifiers::CTRL);
    let alt = kb.modifiers.contains(Modifiers::ALT);
    let shift = kb.modifiers.contains(Modifiers::SHIFT);

    // Plain Ctrl+S saves when the editor is focused. It is deliberately
    // NOT a global keybind: registered Ctrl-combos are consumed before
    // terminal capture, which would break XOFF/nano in terminal panes.
    if ctrl && !alt && !shift && matches!(kb.key, Key::Char('s') | Key::Char('S')) {
        if st.editors.contains_key(&pane_id) {
            return Some(crate::state::dispatch(st, "editor.save"));
        }
        return None;
    }

    let editor = st.editors.get_mut(&pane_id)?;
    let page = page_step(editor.grid.rows());

    let changed = match kb.key {
        Key::ArrowLeft if ctrl => editor.apply(|b| {
            b.move_word_left(shift);
            Damage::None
        }),
        Key::ArrowRight if ctrl => editor.apply(|b| {
            b.move_word_right(shift);
            Damage::None
        }),
        Key::ArrowLeft => editor.apply(|b| {
            b.move_left(shift);
            Damage::None
        }),
        Key::ArrowRight => editor.apply(|b| {
            b.move_right(shift);
            Damage::None
        }),
        // Ctrl+Up/Down scroll the viewport without moving the cursor
        // (VS Code behavior).
        Key::ArrowUp if ctrl => editor.scroll_by(-1),
        Key::ArrowDown if ctrl => editor.scroll_by(1),
        Key::ArrowUp => editor.apply(|b| {
            b.move_up(shift);
            Damage::None
        }),
        Key::ArrowDown => editor.apply(|b| {
            b.move_down(shift);
            Damage::None
        }),
        Key::PageUp => editor.apply(|b| {
            b.move_page(-page, shift);
            Damage::None
        }),
        Key::PageDown => editor.apply(|b| {
            b.move_page(page, shift);
            Damage::None
        }),
        Key::Home if ctrl => editor.apply(|b| {
            b.move_doc_start(shift);
            Damage::None
        }),
        Key::End if ctrl => editor.apply(|b| {
            b.move_doc_end(shift);
            Damage::None
        }),
        Key::Home => editor.apply(|b| {
            b.move_home(shift);
            Damage::None
        }),
        Key::End => editor.apply(|b| {
            b.move_end(shift);
            Damage::None
        }),
        Key::Enter if !ctrl && !alt => editor.apply(|b| b.insert_newline()),
        Key::Tab if !ctrl && !alt && !shift => {
            editor.apply(|b| b.insert_typed(&" ".repeat(TAB_SPACES)))
        }
        Key::Backspace => editor.apply(|b| b.backspace(ctrl)),
        Key::Delete => editor.apply(|b| b.delete_forward(ctrl)),
        Key::Char('a') | Key::Char('A') if ctrl && !shift && !alt => editor.apply(|b| {
            b.select_all();
            Damage::None
        }),
        // Clipboard. Copy/cut consume the key even without a selection
        // so nothing leaks toward other handlers.
        Key::Char('c') | Key::Char('C') if ctrl && !shift && !alt => {
            if let Some(text) = editor.buffer.selected_text() {
                if let Err(e) = st.clipboard.write_text(&text) {
                    log::warn!("editor copy: clipboard write failed: {e}");
                }
            }
            false
        }
        Key::Char('x') | Key::Char('X') if ctrl && !shift && !alt => {
            match editor.buffer.selected_text() {
                Some(text) => {
                    if let Err(e) = st.clipboard.write_text(&text) {
                        log::warn!("editor cut: clipboard write failed: {e}");
                    }
                    editor.apply(|b| b.delete_selection())
                }
                None => false,
            }
        }
        Key::Char('v') | Key::Char('V') if ctrl && !shift && !alt => {
            match st.clipboard.read_text() {
                Ok(text) if !text.is_empty() => editor.apply(|b| b.insert_str(&text)),
                _ => false,
            }
        }
        // Undo / redo.
        Key::Char('z') | Key::Char('Z') if ctrl && !shift && !alt => editor.apply(|b| b.undo()),
        Key::Char('y') | Key::Char('Y') if ctrl && !shift && !alt => editor.apply(|b| b.redo()),
        Key::Char('z') | Key::Char('Z') if ctrl && shift && !alt => editor.apply(|b| b.redo()),
        _ if !ctrl && !alt => {
            let text = insert_text_for(kb)?;
            editor.apply(|b| b.insert_typed(&text))
        }
        _ => return None,
    };
    Some(changed)
}

/// Viewport grid cell under local pixel coordinates, using the
/// renderer's published cell metrics. Signed so positions outside the
/// element (drags past an edge) still resolve; `None` before the first
/// paint publishes metrics.
fn editor_cell_from_local(local_x: f32, local_y: f32) -> Option<(isize, isize)> {
    let cell_w = unshit::core::cell_grid::CellGrid::global_cell_w();
    let cell_h = unshit::core::cell_grid::CellGrid::global_cell_h();
    if cell_w <= 0.0 || cell_h <= 0.0 {
        return None;
    }
    Some((
        (local_y / cell_h).floor() as isize,
        (local_x / cell_w).floor() as isize,
    ))
}

/// Select all of `line` including its newline (VS Code gutter click /
/// triple-click). The last line selects to its end instead.
fn select_line_at(b: &mut crate::editor::EditorBuffer, line: usize) {
    use crate::editor::Position;
    b.set_cursor(Position { line, col: 0 }, false);
    let end = if line + 1 < b.line_count() {
        Position {
            line: line + 1,
            col: 0,
        }
    } else {
        Position {
            line,
            col: b.line(line).map_or(0, str::len),
        }
    };
    b.set_cursor(end, true);
}

/// Handle a left press on editor viewport cell (`row`, `col_cell`):
/// places the cursor, extends on Shift, selects the word / line on the
/// second / third consecutive press (sharing the terminal's click
/// tracker — pane ids never collide), and selects the whole line from
/// the gutter. Returns `true` when the grid changed.
pub(crate) fn handle_editor_mouse_down(
    st: &mut crate::state::AppState,
    pane_id: u32,
    row: isize,
    col_cell: isize,
    shift: bool,
    now: std::time::Instant,
) -> bool {
    use crate::editor::Damage;

    let Some(editor) = st.editors.get(&pane_id) else {
        return false;
    };
    let gutter = editor.gutter_cells() as isize;
    let pos = editor.position_at_cell(row, col_cell);

    if col_cell < gutter {
        st.terminal_click = None;
        let editor = st.editors.get_mut(&pane_id).expect("checked above");
        return editor.apply(|b| {
            select_line_at(b, pos.line);
            Damage::None
        });
    }

    let cell = (pos.line as u64, pos.col);
    let count = match st.terminal_click {
        Some(prev)
            if prev.pane == pane_id
                && prev.cell == cell
                && now.duration_since(prev.at).as_millis() <= crate::state::MULTI_CLICK_MS =>
        {
            prev.count % 3 + 1
        }
        _ => 1,
    };
    st.terminal_click = Some(crate::state::TerminalClick {
        pane: pane_id,
        at: now,
        cell,
        count,
    });

    let editor = st.editors.get_mut(&pane_id).expect("checked above");
    editor.apply(|b| {
        if shift {
            b.set_cursor(pos, true);
        } else {
            match count {
                2 => b.select_word_at(pos),
                3 => select_line_at(b, pos.line),
                _ => b.set_cursor(pos, false),
            }
        }
        Damage::None
    })
}

/// Extend the selection to the cell under a drag. `apply` keeps the
/// cursor visible, so dragging past an edge auto-scrolls the viewport.
pub(crate) fn handle_editor_drag(
    st: &mut crate::state::AppState,
    pane_id: u32,
    row: isize,
    col_cell: isize,
) -> bool {
    use crate::editor::Damage;
    let Some(editor) = st.editors.get_mut(&pane_id) else {
        return false;
    };
    let pos = editor.position_at_cell(row, col_cell);
    editor.apply(|b| {
        b.set_cursor(pos, true);
        Damage::None
    })
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
                let changed = mutate_with(&kbd_shared, |st| {
                    let changed = handle_editor_key(st, kbd_pane.0, kb)?;
                    if changed {
                        crate::state::sync_editor_pane_title(st, kbd_pane.0);
                    }
                    Some(changed)
                });
                match changed {
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
                // Shift+wheel scrolls horizontally (VS Code / xterm
                // convention), in character steps against the cell width.
                if se.modifiers.contains(Modifiers::SHIFT) {
                    let cell_w = unshit::core::cell_grid::CellGrid::global_cell_w().max(1.0);
                    let chars = (se.delta_y / cell_w).round() as isize;
                    let grid = mutate_with(&scroll_shared, |st| {
                        let editor = st.editors.get_mut(&scroll_pane.0)?;
                        let step = if chars == 0 {
                            if se.delta_y > 0.0 {
                                -1
                            } else if se.delta_y < 0.0 {
                                1
                            } else {
                                return None;
                            }
                        } else {
                            -chars
                        };
                        editor.scroll_h_by(step).then(|| editor.grid.clone())
                    });
                    return Some(Box::new(unshit::app::app::ScrollGridPatch {
                        grid,
                        animation: None,
                    }));
                }
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

    // -- Mouse: cursor placement and selection ---------------------------
    // Registered for every editor pane with a grid (like terminal
    // selection handlers) so clicking an inactive pane both focuses it
    // and places the cursor in one press.
    let down_shared = shared.clone();
    let down_pane = pane_id;
    grid_el = grid_el.on(
        EventType::MouseDown,
        move |event: &Event| -> Option<Box<dyn std::any::Any>> {
            let Event::Mouse(me) = event else {
                return None;
            };
            if me.kind != MouseEventKind::Down || me.button != MouseButton::Left {
                return None;
            }
            let shift = me.modifiers.contains(Modifiers::SHIFT);
            let (row, col) = editor_cell_from_local(me.local_x, me.local_y)?;
            let changed = mutate_with(&down_shared, |st| {
                // Focus on press, mirroring terminal panes: a click-drag
                // suppresses the trailing click, so on_click alone would
                // leave a selection stranded in an inactive pane.
                let ws_idx = st.active_workspace;
                crate::state::dispatch(st, &format!("terminal.focus:{}:{}", ws_idx, down_pane.0));
                handle_editor_mouse_down(
                    st,
                    down_pane.0,
                    row,
                    col,
                    shift,
                    std::time::Instant::now(),
                )
            });
            changed.then(|| Box::new(unshit::core::event::RequestRebuild) as Box<dyn std::any::Any>)
        },
    );

    let drag_shared = shared.clone();
    let drag_pane = pane_id;
    grid_el = grid_el.on_drag(move |ev| {
        if ev.button != MouseButton::Left {
            return;
        }
        if matches!(ev.phase, DragPhase::Start | DragPhase::Update) {
            if let Some((row, col)) = editor_cell_from_local(ev.local_x, ev.local_y) {
                mutate_with(&drag_shared, |st| {
                    handle_editor_drag(st, drag_pane.0, row, col);
                });
            }
        }
    });

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

    // -- keystroke handling -------------------------------------------------

    use unshit::core::event::{KeyEventKind, KeyboardEvent};

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

    fn char_key(c: char) -> KeyboardEvent {
        KeyboardEvent {
            kind: KeyEventKind::Pressed,
            key: Key::Char(c),
            modifiers: Modifiers::empty(),
            text: Some(c.to_string()),
        }
    }

    fn editor_state() -> (crate::state::AppState, std::path::PathBuf) {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tm-editor-keys-{}-{}.txt",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, "hello\nworld").expect("write temp file");
        let mut state = seed_state();
        let editor = crate::editor::EditorPane::open(&path, 10, 40).expect("open editor");
        state.editors.insert(1, editor);
        (state, path)
    }

    fn grid_row_text(state: &crate::state::AppState, row: usize) -> String {
        let grid = &state.editors.get(&1).unwrap().grid;
        (0..grid.cols())
            .map(|c| {
                let ch = grid.get_cell(row, c).map(|cell| cell.ch).unwrap_or('\0');
                if ch == '\0' {
                    ' '
                } else {
                    ch
                }
            })
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn typing_chars_lands_in_buffer_and_grid() {
        let (mut state, path) = editor_state();
        for c in ['a', 'b', 'c'] {
            assert_eq!(handle_editor_key(&mut state, 1, &char_key(c)), Some(true));
        }
        let editor = state.editors.get(&1).unwrap();
        assert_eq!(editor.buffer.line(0), Some("abchello"));
        assert!(editor.dirty);
        assert_eq!(grid_row_text(&state, 0), "  1 abchello");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn dead_key_composed_text_is_preferred_over_key_char() {
        let (mut state, path) = editor_state();
        // US-International dead-key composition: key reports the base
        // char but text carries the composed character.
        let ev = KeyboardEvent {
            kind: KeyEventKind::Pressed,
            key: Key::Char('e'),
            modifiers: Modifiers::empty(),
            text: Some("é".to_string()),
        };
        assert_eq!(handle_editor_key(&mut state, 1, &ev), Some(true));
        assert_eq!(
            state.editors.get(&1).unwrap().buffer.line(0),
            Some("éhello")
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn enter_backspace_delete_edit_lines() {
        let (mut state, path) = editor_state();
        handle_editor_key(&mut state, 1, &key_mod(Key::ArrowRight, Modifiers::CTRL));
        handle_editor_key(&mut state, 1, &key(Key::Enter));
        assert_eq!(state.editors.get(&1).unwrap().buffer.line_count(), 3);
        handle_editor_key(&mut state, 1, &key(Key::Backspace));
        assert_eq!(state.editors.get(&1).unwrap().buffer.line_count(), 2);
        handle_editor_key(&mut state, 1, &key(Key::Delete));
        assert_eq!(
            state.editors.get(&1).unwrap().buffer.line(0),
            Some("helloworld")
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn ctrl_a_selects_all_and_typing_replaces() {
        let (mut state, path) = editor_state();
        handle_editor_key(&mut state, 1, &key_mod(Key::Char('a'), Modifiers::CTRL));
        assert!(state.editors.get(&1).unwrap().buffer.selection().is_some());
        handle_editor_key(&mut state, 1, &char_key('x'));
        let editor = state.editors.get(&1).unwrap();
        assert_eq!(editor.buffer.to_text(), "x");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn shift_arrows_select_and_grid_paints_selection() {
        let (mut state, path) = editor_state();
        handle_editor_key(&mut state, 1, &key_mod(Key::ArrowRight, Modifiers::SHIFT));
        handle_editor_key(&mut state, 1, &key_mod(Key::ArrowRight, Modifiers::SHIFT));
        let editor = state.editors.get(&1).unwrap();
        assert_eq!(editor.buffer.selected_text().as_deref(), Some("he"));
        // Selected cells carry a non-transparent background.
        let bg = editor.grid.get_cell(0, 4).unwrap().bg;
        assert_ne!(bg.a, 0, "selection background should be painted");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn dirty_marker_appears_in_pane_title() {
        let (mut state, path) = editor_state();
        // Seed a pane entry matching the editor pane id so titles sync.
        state.panes[0][0].id = PaneId(1);
        handle_editor_key(&mut state, 1, &char_key('z'));
        crate::state::sync_editor_pane_title(&mut state, 1);
        assert!(state.panes[0][0].title.starts_with('\u{25CF}'));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn unrelated_keys_are_not_consumed() {
        let (mut state, path) = editor_state();
        assert_eq!(
            handle_editor_key(&mut state, 1, &key_mod(Key::Char('p'), Modifiers::CTRL)),
            None
        );
        assert_eq!(handle_editor_key(&mut state, 1, &key(Key::Escape)), None);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn ctrl_z_undoes_typing_run_and_ctrl_y_redoes() {
        let (mut state, path) = editor_state();
        for c in ['a', 'b', 'c'] {
            handle_editor_key(&mut state, 1, &char_key(c));
        }
        assert_eq!(
            handle_editor_key(&mut state, 1, &key_mod(Key::Char('z'), Modifiers::CTRL)),
            Some(true)
        );
        let editor = state.editors.get(&1).unwrap();
        assert_eq!(editor.buffer.line(0), Some("hello"));
        assert!(!editor.dirty, "undo back to pristine clears dirty");
        assert_eq!(
            handle_editor_key(&mut state, 1, &key_mod(Key::Char('y'), Modifiers::CTRL)),
            Some(true)
        );
        let editor = state.editors.get(&1).unwrap();
        assert_eq!(editor.buffer.line(0), Some("abchello"));
        assert!(editor.dirty);
        // Ctrl+Shift+Z is redo too (after another undo).
        handle_editor_key(&mut state, 1, &key_mod(Key::Char('z'), Modifiers::CTRL));
        handle_editor_key(
            &mut state,
            1,
            &key_mod(Key::Char('Z'), Modifiers::CTRL | Modifiers::SHIFT),
        );
        assert_eq!(
            state.editors.get(&1).unwrap().buffer.line(0),
            Some("abchello")
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn undo_with_empty_history_consumes_without_change() {
        let (mut state, path) = editor_state();
        assert_eq!(
            handle_editor_key(&mut state, 1, &key_mod(Key::Char('z'), Modifiers::CTRL)),
            Some(false)
        );
        let _ = std::fs::remove_file(path);
    }

    // -- mouse handling -----------------------------------------------------

    #[test]
    fn editor_pane_registers_mouse_handlers_even_when_inactive() {
        let (shared, path) = shared_with_editor();
        let grids = grids_for(&shared);
        let el = build_editor_pane_body(PaneId(1), false, 13, &shared, &grids);
        let grid_el = &el.children[0];
        assert!(
            grid_el
                .handlers
                .iter()
                .any(|(et, _)| *et == EventType::MouseDown),
            "editor grid must register a MouseDown handler"
        );
        assert!(
            grid_el.on_drag.is_some(),
            "editor grid must register an on_drag handler"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn click_places_cursor_gutter_aware() {
        let (mut state, path) = editor_state();
        let gutter = state.editors.get(&1).unwrap().gutter_cells() as isize;
        let t0 = std::time::Instant::now();
        assert!(handle_editor_mouse_down(
            &mut state,
            1,
            1,
            gutter + 3,
            false,
            t0
        ));
        let editor = state.editors.get(&1).unwrap();
        assert_eq!(
            editor.buffer.cursor(),
            crate::editor::Position { line: 1, col: 3 }
        );
        assert!(editor.buffer.selection().is_none());
        assert_eq!(editor.grid.cursor_row(), 1);
        assert_eq!(editor.grid.cursor_col(), (gutter + 3) as usize);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn double_click_selects_word_triple_selects_line() {
        let (mut state, path) = editor_state();
        let gutter = state.editors.get(&1).unwrap().gutter_cells() as isize;
        let t0 = std::time::Instant::now();
        handle_editor_mouse_down(&mut state, 1, 0, gutter + 2, false, t0);
        handle_editor_mouse_down(&mut state, 1, 0, gutter + 2, false, t0);
        assert_eq!(
            state
                .editors
                .get(&1)
                .unwrap()
                .buffer
                .selected_text()
                .as_deref(),
            Some("hello")
        );
        handle_editor_mouse_down(&mut state, 1, 0, gutter + 2, false, t0);
        assert_eq!(
            state
                .editors
                .get(&1)
                .unwrap()
                .buffer
                .selected_text()
                .as_deref(),
            Some("hello\n"),
            "third click promotes to whole-line selection"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn slow_second_click_does_not_promote_to_word() {
        let (mut state, path) = editor_state();
        let gutter = state.editors.get(&1).unwrap().gutter_cells() as isize;
        let t0 = std::time::Instant::now();
        handle_editor_mouse_down(&mut state, 1, 0, gutter + 2, false, t0);
        let late = t0 + std::time::Duration::from_millis(crate::state::MULTI_CLICK_MS as u64 + 50);
        handle_editor_mouse_down(&mut state, 1, 0, gutter + 2, false, late);
        assert!(state.editors.get(&1).unwrap().buffer.selection().is_none());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn gutter_click_selects_whole_line() {
        let (mut state, path) = editor_state();
        let t0 = std::time::Instant::now();
        assert!(handle_editor_mouse_down(&mut state, 1, 0, 1, false, t0));
        assert_eq!(
            state
                .editors
                .get(&1)
                .unwrap()
                .buffer
                .selected_text()
                .as_deref(),
            Some("hello\n")
        );
        // Last line has no newline to include.
        handle_editor_mouse_down(&mut state, 1, 1, 0, false, t0);
        assert_eq!(
            state
                .editors
                .get(&1)
                .unwrap()
                .buffer
                .selected_text()
                .as_deref(),
            Some("world")
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn shift_click_and_drag_extend_selection() {
        let (mut state, path) = editor_state();
        let gutter = state.editors.get(&1).unwrap().gutter_cells() as isize;
        let t0 = std::time::Instant::now();
        handle_editor_mouse_down(&mut state, 1, 0, gutter + 1, false, t0);
        handle_editor_mouse_down(&mut state, 1, 0, gutter + 4, true, t0);
        assert_eq!(
            state
                .editors
                .get(&1)
                .unwrap()
                .buffer
                .selected_text()
                .as_deref(),
            Some("ell")
        );
        // Drag further onto the second line.
        assert!(handle_editor_drag(&mut state, 1, 1, gutter + 2));
        assert_eq!(
            state
                .editors
                .get(&1)
                .unwrap()
                .buffer
                .selected_text()
                .as_deref(),
            Some("ello\nwo")
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn drag_below_document_scrolls_and_clamps_to_end() {
        let path =
            std::env::temp_dir().join(format!("tm-editor-dragscroll-{}.txt", std::process::id()));
        let text: Vec<String> = (0..40).map(|i| format!("l{}", i)).collect();
        std::fs::write(&path, text.join("\n")).expect("write temp file");
        let mut state = seed_state();
        let editor = crate::editor::EditorPane::open(&path, 10, 40).expect("open editor");
        state.editors.insert(1, editor);
        let t0 = std::time::Instant::now();
        handle_editor_mouse_down(&mut state, 1, 0, 10, false, t0);
        // Drag far past the bottom edge: viewport follows the cursor.
        assert!(handle_editor_drag(&mut state, 1, 99, 10));
        let editor = state.editors.get(&1).unwrap();
        assert_eq!(editor.buffer.cursor().line, 39);
        assert_eq!(editor.top_line, 30);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn shift_wheel_scrolls_horizontally() {
        use unshit::core::cell_grid::CellGrid;
        let path =
            std::env::temp_dir().join(format!("tm-editor-hwheel-{}.txt", std::process::id()));
        std::fs::write(&path, "abcdefghijklmnopqrstuvwxyz0123456789").expect("write temp file");
        CellGrid::publish_cell_metrics(10.0, 20.0);
        let shared: SharedState = {
            let mut state = seed_state();
            let editor = crate::editor::EditorPane::open(&path, 4, 20).expect("open editor");
            state.editors.insert(1, editor);
            Arc::new(Mutex::new(state))
        };
        let grids = grids_for(&shared);
        let el = build_editor_pane_body(PaneId(1), true, 13, &shared, &grids);
        let handler = el.children[0]
            .handlers
            .iter()
            .find(|(et, _)| *et == EventType::Scroll)
            .map(|(_, h)| h.clone())
            .expect("Scroll handler must be present");
        // Wheel down (delta_y < 0) with Shift held: scroll right.
        let ev = Event::Scroll(unshit::core::event::ScrollEvent {
            delta_x: 0.0,
            delta_y: -30.0,
            x: 0.0,
            y: 0.0,
            modifiers: Modifiers::SHIFT,
            ..Default::default()
        });
        let patch = (handler)(&ev)
            .expect("handler must return a patch")
            .downcast::<unshit::app::app::ScrollGridPatch>()
            .expect("must be a ScrollGridPatch");
        assert!(patch.grid.is_some(), "changed offset republishes the grid");
        assert_eq!(shared.lock().unwrap().editors.get(&1).unwrap().h_offset, 3);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn typing_at_viewport_bottom_scrolls_cursor_into_view() {
        let path =
            std::env::temp_dir().join(format!("tm-editor-scrolltype-{}.txt", std::process::id()));
        let text: Vec<String> = (0..40).map(|i| format!("l{}", i)).collect();
        std::fs::write(&path, text.join("\n")).expect("write temp file");
        let mut state = seed_state();
        let editor = crate::editor::EditorPane::open(&path, 10, 40).expect("open editor");
        state.editors.insert(1, editor);
        handle_editor_key(&mut state, 1, &key_mod(Key::End, Modifiers::CTRL));
        let editor = state.editors.get(&1).unwrap();
        assert_eq!(editor.buffer.cursor().line, 39);
        assert_eq!(editor.top_line, 30, "viewport follows the cursor");
        assert!(editor.grid.cursor_visible());
        assert_eq!(editor.grid.cursor_row(), 9);
        let _ = std::fs::remove_file(path);
    }
}
