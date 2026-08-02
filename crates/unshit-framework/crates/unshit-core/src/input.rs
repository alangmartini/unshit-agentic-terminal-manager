//! Shared text-input editing logic used by both the app event loop
//! and the test harness.

use crate::element::{InputState, InputType};
use crate::event::Key;

/// Ordered byte range of the active selection, or `None` when there is
/// no selection or it is collapsed.
pub fn selection_range(state: &InputState) -> Option<(usize, usize)> {
    let anchor = state.selection_anchor?;
    if anchor == state.cursor_pos {
        return None;
    }
    Some(if anchor < state.cursor_pos {
        (anchor, state.cursor_pos)
    } else {
        (state.cursor_pos, anchor)
    })
}

/// Select the entire value and place the cursor at the end (Ctrl+A).
/// Returns `true` if the selection or cursor changed.
pub fn select_all(state: &mut InputState) -> bool {
    let changed = state.selection_anchor != Some(0) || state.cursor_pos != state.value.len();
    state.selection_anchor = Some(0);
    state.cursor_pos = state.value.len();
    changed
}

/// Remove the selected text, collapsing the cursor to the selection start.
/// Returns `true` when text was removed.
pub fn delete_selection(state: &mut InputState) -> bool {
    let Some((start, end)) = selection_range(state) else {
        state.selection_anchor = None;
        return false;
    };
    state.value.drain(start..end);
    state.cursor_pos = start;
    state.selection_anchor = None;
    true
}

/// Apply a key action to an InputState. Returns `true` if the value
/// changed (either content or cursor position).
pub fn apply_key(state: &mut InputState, key: &Key) -> bool {
    apply_key_with_mods(state, key, false, false)
}

/// Apply a key action with modifier context: `shift` extends the selection
/// on movement keys, `ctrl` switches movement/deletion to word granularity.
/// Returns `true` if anything observable changed (content, cursor, or
/// selection).
pub fn apply_key_with_mods(state: &mut InputState, key: &Key, shift: bool, ctrl: bool) -> bool {
    let old_len = state.value.len();
    let old_cursor = state.cursor_pos;
    let old_numeric = state.numeric_value;
    let old_anchor = state.selection_anchor;

    match state.input_type {
        InputType::Number => match key {
            Key::ArrowUp => {
                state.numeric_value = (state.numeric_value + state.step).min(state.max);
                state.value = format_numeric(state.numeric_value);
                state.cursor_pos = state.value.len();
                return true;
            }
            Key::ArrowDown => {
                state.numeric_value = (state.numeric_value - state.step).max(state.min);
                state.value = format_numeric(state.numeric_value);
                state.cursor_pos = state.value.len();
                return true;
            }
            _ => {}
        },
        InputType::Range => match key {
            Key::ArrowUp | Key::ArrowRight => {
                state.numeric_value = (state.numeric_value + state.step).min(state.max);
                state.value = format_numeric(state.numeric_value);
                return true;
            }
            Key::ArrowDown | Key::ArrowLeft => {
                state.numeric_value = (state.numeric_value - state.step).max(state.min);
                state.value = format_numeric(state.numeric_value);
                return true;
            }
            _ => return false,
        },
        InputType::Checkbox | InputType::Radio | InputType::Hidden => {
            // These types do not respond to key-based editing.
            return false;
        }
        InputType::Text | InputType::Password => {}
    }

    match key {
        Key::Backspace => {
            if !delete_selection(state) && state.cursor_pos > 0 {
                let prev = if ctrl {
                    prev_word_boundary(&state.value, state.cursor_pos)
                } else {
                    prev_char_boundary(&state.value, state.cursor_pos)
                };
                state.value.drain(prev..state.cursor_pos);
                state.cursor_pos = prev;
            }
        }
        Key::Delete => {
            if !delete_selection(state) && state.cursor_pos < state.value.len() {
                let next = if ctrl {
                    next_word_boundary(&state.value, state.cursor_pos)
                } else {
                    next_char_boundary(&state.value, state.cursor_pos)
                };
                state.value.drain(state.cursor_pos..next);
            }
        }
        Key::ArrowLeft => {
            if !shift && !ctrl {
                if let Some((start, _)) = selection_range(state) {
                    // Plain arrow with a selection collapses to its edge.
                    state.cursor_pos = start;
                    state.selection_anchor = None;
                } else if state.cursor_pos > 0 {
                    state.cursor_pos = prev_char_boundary(&state.value, state.cursor_pos);
                    state.selection_anchor = None;
                }
            } else {
                let target = if ctrl {
                    prev_word_boundary(&state.value, state.cursor_pos)
                } else {
                    prev_char_boundary(&state.value, state.cursor_pos)
                };
                move_cursor(state, target, shift);
            }
        }
        Key::ArrowRight => {
            if !shift && !ctrl {
                if let Some((_, end)) = selection_range(state) {
                    state.cursor_pos = end;
                    state.selection_anchor = None;
                } else if state.cursor_pos < state.value.len() {
                    state.cursor_pos = next_char_boundary(&state.value, state.cursor_pos);
                    state.selection_anchor = None;
                }
            } else {
                let target = if ctrl {
                    next_word_boundary(&state.value, state.cursor_pos)
                } else {
                    next_char_boundary(&state.value, state.cursor_pos)
                };
                move_cursor(state, target, shift);
            }
        }
        Key::Home => {
            move_cursor(state, 0, shift);
        }
        Key::End => {
            move_cursor(state, state.value.len(), shift);
        }
        _ => {}
    }

    // After text editing on Number, try to sync numeric_value.
    if state.input_type == InputType::Number && state.value.len() != old_len {
        if let Ok(v) = state.value.parse::<f32>() {
            state.numeric_value = v.clamp(state.min, state.max);
        }
    }

    state.value.len() != old_len
        || state.cursor_pos != old_cursor
        || state.numeric_value != old_numeric
        || state.selection_anchor != old_anchor
}

/// Move the cursor to `target`, extending the selection when `extend` is
/// set (anchoring at the current cursor if no selection exists) and
/// clearing it otherwise.
fn move_cursor(state: &mut InputState, target: usize, extend: bool) {
    if extend {
        if state.selection_anchor.is_none() {
            state.selection_anchor = Some(state.cursor_pos);
        }
        state.cursor_pos = target;
        // A selection collapsed back onto its anchor is no selection.
        if state.selection_anchor == Some(state.cursor_pos) {
            state.selection_anchor = None;
        }
    } else {
        state.cursor_pos = target;
        state.selection_anchor = None;
    }
}

/// Insert text at the current cursor position. For Number inputs, only
/// numeric characters (digits, minus, decimal point) are accepted.
pub fn insert_text_filtered(state: &mut InputState, text: &str) -> bool {
    match state.input_type {
        InputType::Number => {
            let filtered: String =
                text.chars().filter(|&c| c.is_ascii_digit() || c == '-' || c == '.').collect();
            if filtered.is_empty() {
                return false;
            }
            insert_text(state, &filtered);
            // Sync numeric value without clamping (clamping happens on blur/submit).
            if let Ok(v) = state.value.parse::<f32>() {
                state.numeric_value = v;
            }
            true
        }
        _ => {
            insert_text(state, text);
            true
        }
    }
}

/// Clamp a Number input's value to [min, max] and sync string representation.
/// Call this on blur or submit.
pub fn clamp_number_input(state: &mut InputState) {
    if state.input_type != InputType::Number {
        return;
    }
    if let Ok(v) = state.value.parse::<f32>() {
        let clamped = v.clamp(state.min, state.max);
        state.numeric_value = clamped;
        state.value = format_numeric(clamped);
        state.cursor_pos = state.value.len();
    }
}

/// Format a float for display, stripping unnecessary trailing zeros.
fn format_numeric(v: f32) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{}", v)
    }
}

/// Insert text at the current cursor position and advance the cursor.
/// An active selection is replaced by the inserted text.
pub fn insert_text(state: &mut InputState, text: &str) {
    delete_selection(state);
    state.value.insert_str(state.cursor_pos, text);
    state.cursor_pos += text.len();
}

fn prev_char_boundary(s: &str, pos: usize) -> usize {
    s[..pos].char_indices().rev().next().map(|(i, _)| i).unwrap_or(0)
}

fn next_char_boundary(s: &str, pos: usize) -> usize {
    s[pos..].char_indices().nth(1).map(|(i, _)| pos + i).unwrap_or(s.len())
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Byte offset of the start of the word left of `pos` (Ctrl+Left /
/// Ctrl+Backspace target): skip whitespace, then take the run of
/// same-class characters (word chars vs punctuation).
fn prev_word_boundary(s: &str, pos: usize) -> usize {
    let chars: Vec<(usize, char)> = s[..pos].char_indices().collect();
    let mut i = chars.len();
    while i > 0 && chars[i - 1].1.is_whitespace() {
        i -= 1;
    }
    if i == 0 {
        return 0;
    }
    let class = is_word_char(chars[i - 1].1);
    while i > 0 && !chars[i - 1].1.is_whitespace() && is_word_char(chars[i - 1].1) == class {
        i -= 1;
    }
    if i == 0 {
        0
    } else {
        chars[i].0
    }
}

/// Byte offset of the start of the next word right of `pos` (Ctrl+Right /
/// Ctrl+Delete target): skip the current same-class run, then any
/// whitespace after it.
fn next_word_boundary(s: &str, pos: usize) -> usize {
    let chars: Vec<(usize, char)> = s[pos..].char_indices().collect();
    let mut i = 0;
    if i < chars.len() && !chars[i].1.is_whitespace() {
        let class = is_word_char(chars[i].1);
        while i < chars.len() && !chars[i].1.is_whitespace() && is_word_char(chars[i].1) == class {
            i += 1;
        }
    }
    while i < chars.len() && chars[i].1.is_whitespace() {
        i += 1;
    }
    if i >= chars.len() {
        s.len()
    } else {
        pos + chars[i].0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state(value: &str, cursor_pos: usize) -> InputState {
        InputState { value: value.into(), cursor_pos, ..InputState::default() }
    }

    #[test]
    fn insert_at_start() {
        let mut s = make_state("ello", 0);
        insert_text(&mut s, "h");
        assert_eq!(s.value, "hello");
        assert_eq!(s.cursor_pos, 1);
    }

    #[test]
    fn insert_at_end() {
        let mut s = make_state("hell", 4);
        insert_text(&mut s, "o");
        assert_eq!(s.value, "hello");
        assert_eq!(s.cursor_pos, 5);
    }

    #[test]
    fn backspace_removes_char() {
        let mut s = make_state("hello", 5);
        assert!(apply_key(&mut s, &Key::Backspace));
        assert_eq!(s.value, "hell");
        assert_eq!(s.cursor_pos, 4);
    }

    #[test]
    fn backspace_at_start_is_noop() {
        let mut s = make_state("hello", 0);
        assert!(!apply_key(&mut s, &Key::Backspace));
        assert_eq!(s.value, "hello");
    }

    #[test]
    fn delete_removes_char() {
        let mut s = make_state("hello", 0);
        assert!(apply_key(&mut s, &Key::Delete));
        assert_eq!(s.value, "ello");
        assert_eq!(s.cursor_pos, 0);
    }

    #[test]
    fn delete_at_end_is_noop() {
        let mut s = make_state("hello", 5);
        assert!(!apply_key(&mut s, &Key::Delete));
    }

    #[test]
    fn arrow_left_right() {
        let mut s = make_state("abc", 3);
        apply_key(&mut s, &Key::ArrowLeft);
        assert_eq!(s.cursor_pos, 2);
        apply_key(&mut s, &Key::ArrowRight);
        assert_eq!(s.cursor_pos, 3);
    }

    #[test]
    fn home_end() {
        let mut s = make_state("abc", 1);
        apply_key(&mut s, &Key::Home);
        assert_eq!(s.cursor_pos, 0);
        apply_key(&mut s, &Key::End);
        assert_eq!(s.cursor_pos, 3);
    }

    #[test]
    fn unicode_backspace() {
        // "he" + e-acute (2 bytes) + "lo"
        let mut s = make_state("he\u{00e9}lo", 5);
        // cursor after 'l', backspace removes 'l'
        apply_key(&mut s, &Key::Backspace);
        assert_eq!(s.value, "he\u{00e9}o");
        assert_eq!(s.cursor_pos, 4);
    }

    #[test]
    fn select_all_spans_whole_value() {
        let mut s = make_state("hello", 2);
        assert!(select_all(&mut s));
        assert_eq!(selection_range(&s), Some((0, 5)));
        assert_eq!(s.cursor_pos, 5);
        // Selecting all again reports no change.
        assert!(!select_all(&mut s));
    }

    #[test]
    fn select_all_on_empty_value_is_collapsed() {
        let mut s = make_state("", 0);
        select_all(&mut s);
        assert_eq!(selection_range(&s), None);
    }

    #[test]
    fn typing_replaces_selection() {
        let mut s = make_state("hello world", 0);
        select_all(&mut s);
        insert_text(&mut s, "x");
        assert_eq!(s.value, "x");
        assert_eq!(s.cursor_pos, 1);
        assert_eq!(s.selection_anchor, None);
    }

    #[test]
    fn backspace_deletes_selection() {
        let mut s = make_state("hello", 0);
        select_all(&mut s);
        assert!(apply_key(&mut s, &Key::Backspace));
        assert_eq!(s.value, "");
        assert_eq!(s.cursor_pos, 0);
        assert_eq!(s.selection_anchor, None);
    }

    #[test]
    fn delete_deletes_selection() {
        let mut s = make_state("hello", 1);
        s.selection_anchor = Some(4);
        assert!(apply_key(&mut s, &Key::Delete));
        assert_eq!(s.value, "ho");
        assert_eq!(s.cursor_pos, 1);
    }

    #[test]
    fn shift_arrow_extends_and_plain_arrow_collapses() {
        let mut s = make_state("abc", 0);
        apply_key_with_mods(&mut s, &Key::ArrowRight, true, false);
        apply_key_with_mods(&mut s, &Key::ArrowRight, true, false);
        assert_eq!(selection_range(&s), Some((0, 2)));
        // Plain ArrowLeft collapses to the selection start.
        apply_key(&mut s, &Key::ArrowLeft);
        assert_eq!(s.cursor_pos, 0);
        assert_eq!(selection_range(&s), None);
    }

    #[test]
    fn plain_arrow_right_collapses_to_selection_end() {
        let mut s = make_state("abcd", 3);
        s.selection_anchor = Some(1);
        apply_key(&mut s, &Key::ArrowRight);
        assert_eq!(s.cursor_pos, 3);
        assert_eq!(selection_range(&s), None);
    }

    #[test]
    fn shift_home_end_select_to_edges() {
        let mut s = make_state("abcde", 2);
        apply_key_with_mods(&mut s, &Key::Home, true, false);
        assert_eq!(selection_range(&s), Some((0, 2)));
        apply_key_with_mods(&mut s, &Key::End, true, false);
        // Anchor stays at the original cursor while focus moves to the end.
        assert_eq!(selection_range(&s), Some((2, 5)));
    }

    #[test]
    fn shift_arrow_back_onto_anchor_clears_selection() {
        let mut s = make_state("abc", 1);
        apply_key_with_mods(&mut s, &Key::ArrowRight, true, false);
        assert_eq!(selection_range(&s), Some((1, 2)));
        apply_key_with_mods(&mut s, &Key::ArrowLeft, true, false);
        assert_eq!(selection_range(&s), None);
    }

    #[test]
    fn ctrl_arrow_moves_by_word() {
        let mut s = make_state("foo bar_baz  qux", 0);
        apply_key_with_mods(&mut s, &Key::ArrowRight, false, true);
        assert_eq!(s.cursor_pos, 4); // start of "bar_baz"
        apply_key_with_mods(&mut s, &Key::ArrowRight, false, true);
        assert_eq!(s.cursor_pos, 13); // start of "qux"
        apply_key_with_mods(&mut s, &Key::ArrowLeft, false, true);
        assert_eq!(s.cursor_pos, 4);
        apply_key_with_mods(&mut s, &Key::ArrowLeft, false, true);
        assert_eq!(s.cursor_pos, 0);
    }

    #[test]
    fn ctrl_shift_arrow_selects_word() {
        let mut s = make_state("foo bar", 0);
        apply_key_with_mods(&mut s, &Key::ArrowRight, true, true);
        assert_eq!(selection_range(&s), Some((0, 4)));
    }

    #[test]
    fn ctrl_backspace_deletes_word() {
        let mut s = make_state("foo bar", 7);
        assert!(apply_key_with_mods(&mut s, &Key::Backspace, false, true));
        assert_eq!(s.value, "foo ");
        assert_eq!(s.cursor_pos, 4);
    }

    #[test]
    fn ctrl_delete_deletes_to_next_word() {
        let mut s = make_state("foo bar baz", 4);
        assert!(apply_key_with_mods(&mut s, &Key::Delete, false, true));
        assert_eq!(s.value, "foo baz");
        assert_eq!(s.cursor_pos, 4);
    }

    #[test]
    fn paste_over_selection_replaces_it() {
        let mut s = make_state("abcdef", 1);
        s.selection_anchor = Some(5);
        insert_text(&mut s, "XY");
        assert_eq!(s.value, "aXYf");
        assert_eq!(s.cursor_pos, 3);
    }

    #[test]
    fn home_end_clear_selection_without_shift() {
        let mut s = make_state("abc", 1);
        s.selection_anchor = Some(3);
        apply_key(&mut s, &Key::Home);
        assert_eq!(s.cursor_pos, 0);
        assert_eq!(selection_range(&s), None);
    }
}
