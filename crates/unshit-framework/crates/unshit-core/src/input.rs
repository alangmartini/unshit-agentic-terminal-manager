//! Shared text-input editing logic used by both the app event loop
//! and the test harness.

use crate::element::{InputState, InputType};
use crate::event::Key;

/// Apply a key action to an InputState. Returns `true` if the value
/// changed (either content or cursor position).
pub fn apply_key(state: &mut InputState, key: &Key) -> bool {
    let old_len = state.value.len();
    let old_cursor = state.cursor_pos;
    let old_numeric = state.numeric_value;

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
            if state.cursor_pos > 0 {
                let prev = prev_char_boundary(&state.value, state.cursor_pos);
                state.value.drain(prev..state.cursor_pos);
                state.cursor_pos = prev;
            }
        }
        Key::Delete => {
            if state.cursor_pos < state.value.len() {
                let next = next_char_boundary(&state.value, state.cursor_pos);
                state.value.drain(state.cursor_pos..next);
            }
        }
        Key::ArrowLeft => {
            if state.cursor_pos > 0 {
                state.cursor_pos = prev_char_boundary(&state.value, state.cursor_pos);
            }
        }
        Key::ArrowRight => {
            if state.cursor_pos < state.value.len() {
                state.cursor_pos = next_char_boundary(&state.value, state.cursor_pos);
            }
        }
        Key::Home => {
            state.cursor_pos = 0;
        }
        Key::End => {
            state.cursor_pos = state.value.len();
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
pub fn insert_text(state: &mut InputState, text: &str) {
    state.value.insert_str(state.cursor_pos, text);
    state.cursor_pos += text.len();
}

fn prev_char_boundary(s: &str, pos: usize) -> usize {
    s[..pos].char_indices().rev().next().map(|(i, _)| i).unwrap_or(0)
}

fn next_char_boundary(s: &str, pos: usize) -> usize {
    s[pos..].char_indices().nth(1).map(|(i, _)| pos + i).unwrap_or(s.len())
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
}
