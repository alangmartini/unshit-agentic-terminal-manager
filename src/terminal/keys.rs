//! Map framework `Key` types to terminal escape sequences.
//!
//! Translates `KeyboardEvent` values from the unshit event system into the
//! byte sequences a PTY/shell expects. Supports plain characters, modifier
//! combinations (Ctrl, Alt, Shift), arrow keys, function keys, and common
//! special keys (Home, End, Page Up/Down, Delete, etc.).

use unshit::core::event::{Key, KeyEventKind, KeyboardEvent, Modifiers};

/// Encode a `KeyboardEvent` into the byte sequence expected by a terminal.
///
/// Returns `None` for key-up events or keys that have no terminal encoding.
pub fn encode_key(event: &KeyboardEvent) -> Option<Vec<u8>> {
    if event.kind != KeyEventKind::Pressed {
        return None;
    }

    let has_ctrl = event.modifiers.contains(Modifiers::CTRL);
    let has_alt = event.modifiers.contains(Modifiers::ALT);
    let has_shift = event.modifiers.contains(Modifiers::SHIFT);

    match event.key {
        // -- Characters --------------------------------------------------------
        Key::Char(c) => {
            if has_ctrl {
                // Ctrl+a..z produce bytes 0x01..0x1A.
                let lower = c.to_ascii_lowercase();
                if lower.is_ascii_lowercase() {
                    let byte = (lower as u8) - b'a' + 1;
                    return Some(vec![byte]);
                }
            }

            if has_alt {
                // Alt prepends ESC before the character.
                let mut buf = vec![0x1b];
                let ch = if has_shift { c.to_ascii_uppercase() } else { c };
                let mut tmp = [0u8; 4];
                buf.extend_from_slice(ch.encode_utf8(&mut tmp).as_bytes());
                return Some(buf);
            }

            // Plain character. Prefer the `text` field when available (it
            // carries the composed string for dead keys / IME), otherwise
            // fall back to encoding the Key::Char value directly.
            if let Some(ref text) = event.text {
                if !text.is_empty() {
                    return Some(text.as_bytes().to_vec());
                }
            }

            let mut tmp = [0u8; 4];
            let bytes = c.encode_utf8(&mut tmp);
            Some(bytes.as_bytes().to_vec())
        }

        // -- Simple special keys -----------------------------------------------
        Key::Enter => Some(vec![0x0D]),
        Key::Tab => Some(vec![0x09]),
        Key::Backspace => Some(vec![0x7F]),
        Key::Escape => Some(vec![0x1B]),
        Key::Space => {
            if has_ctrl {
                // Ctrl+Space sends NUL.
                Some(vec![0x00])
            } else {
                Some(vec![0x20])
            }
        }

        // -- Arrow keys --------------------------------------------------------
        Key::ArrowUp => Some(encode_modified_key(b'A', has_shift, has_alt, has_ctrl)),
        Key::ArrowDown => Some(encode_modified_key(b'B', has_shift, has_alt, has_ctrl)),
        Key::ArrowRight => Some(encode_modified_key(b'C', has_shift, has_alt, has_ctrl)),
        Key::ArrowLeft => Some(encode_modified_key(b'D', has_shift, has_alt, has_ctrl)),

        // -- Navigation keys ---------------------------------------------------
        Key::Home => Some(encode_modified_key(b'H', has_shift, has_alt, has_ctrl)),
        Key::End => Some(encode_modified_key(b'F', has_shift, has_alt, has_ctrl)),
        Key::PageUp => Some(encode_modified_tilde(b"5", has_shift, has_alt, has_ctrl)),
        Key::PageDown => Some(encode_modified_tilde(b"6", has_shift, has_alt, has_ctrl)),
        Key::Delete => Some(encode_modified_tilde(b"3", has_shift, has_alt, has_ctrl)),
        // Insert is `\x1b[2~`. Shift+Insert is intercepted as paste by the
        // global shortcut before reaching here, so a plain Insert still
        // reaches the shell as the standard CSI-tilde sequence.
        Key::Insert => Some(encode_modified_tilde(b"2", has_shift, has_alt, has_ctrl)),

        // -- Function keys (F1 through F12) ------------------------------------
        Key::F(n) => encode_fkey(n, has_shift, has_alt, has_ctrl),

        Key::Unknown => None,
    }
}

/// Encode a CSI-letter key (\x1b[X) with optional modifier.
///
/// Without modifiers: `\x1b[{letter}`.
/// With modifier `m`: `\x1b[1;{m}{letter}`, where `m` is the xterm modifier
/// code (2 = Shift, 3 = Alt, 5 = Ctrl, and combinations sum accordingly).
fn encode_modified_key(letter: u8, shift: bool, alt: bool, ctrl: bool) -> Vec<u8> {
    let modifier = xterm_modifier(shift, alt, ctrl);
    if modifier == 0 {
        vec![0x1b, b'[', letter]
    } else {
        // \x1b[1;{modifier}{letter}
        let m = modifier.to_string();
        let mut buf = Vec::with_capacity(6 + m.len());
        buf.extend_from_slice(b"\x1b[1;");
        buf.extend_from_slice(m.as_bytes());
        buf.push(letter);
        buf
    }
}

/// Encode a CSI-tilde key (\x1b[N~) with optional modifier.
///
/// Without modifiers: `\x1b[{num}~`.
/// With modifier `m`: `\x1b[{num};{m}~`.
fn encode_modified_tilde(num: &[u8], shift: bool, alt: bool, ctrl: bool) -> Vec<u8> {
    let modifier = xterm_modifier(shift, alt, ctrl);
    if modifier == 0 {
        let mut buf = Vec::with_capacity(4 + num.len());
        buf.extend_from_slice(b"\x1b[");
        buf.extend_from_slice(num);
        buf.push(b'~');
        buf
    } else {
        let m = modifier.to_string();
        let mut buf = Vec::with_capacity(5 + num.len() + m.len());
        buf.extend_from_slice(b"\x1b[");
        buf.extend_from_slice(num);
        buf.push(b';');
        buf.extend_from_slice(m.as_bytes());
        buf.push(b'~');
        buf
    }
}

/// Encode a function key (F1..F12).
///
/// F1..F4 use SS3 sequences (\x1bO{P,Q,R,S}).
/// F5..F12 use CSI tilde sequences with specific numeric codes.
fn encode_fkey(n: u8, shift: bool, alt: bool, ctrl: bool) -> Option<Vec<u8>> {
    let modifier = xterm_modifier(shift, alt, ctrl);

    match n {
        // F1..F4 use SS3 encoding (no modifier variant uses CSI with 1;m).
        1..=4 => {
            let letter = match n {
                1 => b'P',
                2 => b'Q',
                3 => b'R',
                4 => b'S',
                _ => unreachable!(),
            };
            if modifier == 0 {
                Some(vec![0x1b, b'O', letter])
            } else {
                // Modified F1..F4: \x1b[1;{m}{P..S}
                let m = modifier.to_string();
                let mut buf = Vec::with_capacity(6 + m.len());
                buf.extend_from_slice(b"\x1b[1;");
                buf.extend_from_slice(m.as_bytes());
                buf.push(letter);
                Some(buf)
            }
        }
        // F5..F12 use CSI tilde with specific numeric codes.
        5..=12 => {
            let code: &[u8] = match n {
                5 => b"15",
                6 => b"17",
                7 => b"18",
                8 => b"19",
                9 => b"20",
                10 => b"21",
                11 => b"23",
                12 => b"24",
                _ => unreachable!(),
            };
            Some(encode_modified_tilde(code, shift, alt, ctrl))
        }
        _ => None,
    }
}

/// Compute the xterm modifier code.
///
/// The code is 1-based and additive:
///   Shift = 2, Alt = 3, Ctrl = 5
///   Shift+Alt = 4, Shift+Ctrl = 6, Alt+Ctrl = 7, Shift+Alt+Ctrl = 8
///
/// Returns 0 when no modifier is active (caller should use the plain
/// sequence).
fn xterm_modifier(shift: bool, alt: bool, ctrl: bool) -> u8 {
    let mut m: u8 = 1;
    if shift {
        m += 1;
    }
    if alt {
        m += 2;
    }
    if ctrl {
        m += 4;
    }
    if m == 1 {
        0
    } else {
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a simple key-press event for testing.
    pub(super) fn key_event(key: Key, modifiers: Modifiers) -> KeyboardEvent {
        KeyboardEvent {
            key,
            kind: KeyEventKind::Pressed,
            modifiers,
            text: None,
        }
    }

    fn key_event_with_text(key: Key, modifiers: Modifiers, text: &str) -> KeyboardEvent {
        KeyboardEvent {
            key,
            kind: KeyEventKind::Pressed,
            modifiers,
            text: Some(text.to_string()),
        }
    }

    fn key_up(key: Key) -> KeyboardEvent {
        KeyboardEvent {
            key,
            kind: KeyEventKind::Released,
            modifiers: Modifiers::empty(),
            text: None,
        }
    }

    // -- Key-up events are ignored --------------------------------------------

    #[test]
    fn key_up_returns_none() {
        assert!(encode_key(&key_up(Key::Char('a'))).is_none());
        assert!(encode_key(&key_up(Key::Enter)).is_none());
    }

    // -- Plain characters -----------------------------------------------------

    #[test]
    fn plain_char() {
        let result = encode_key(&key_event(Key::Char('a'), Modifiers::empty()));
        assert_eq!(result, Some(vec![b'a']));
    }

    #[test]
    fn char_with_text_field() {
        let result = encode_key(&key_event_with_text(
            Key::Char('a'),
            Modifiers::empty(),
            "hello",
        ));
        assert_eq!(result, Some(b"hello".to_vec()));
    }

    // -- Ctrl+char produces control codes -------------------------------------

    #[test]
    fn ctrl_a() {
        let result = encode_key(&key_event(Key::Char('a'), Modifiers::CTRL));
        assert_eq!(result, Some(vec![0x01]));
    }

    #[test]
    fn ctrl_c() {
        let result = encode_key(&key_event(Key::Char('c'), Modifiers::CTRL));
        assert_eq!(result, Some(vec![0x03]));
    }

    #[test]
    fn ctrl_z() {
        let result = encode_key(&key_event(Key::Char('z'), Modifiers::CTRL));
        assert_eq!(result, Some(vec![0x1A]));
    }

    // -- Alt+char prepends ESC ------------------------------------------------

    #[test]
    fn alt_a() {
        let result = encode_key(&key_event(Key::Char('a'), Modifiers::ALT));
        assert_eq!(result, Some(vec![0x1b, b'a']));
    }

    #[test]
    fn alt_shift_a() {
        let result = encode_key(&key_event(
            Key::Char('a'),
            Modifiers::ALT | Modifiers::SHIFT,
        ));
        assert_eq!(result, Some(vec![0x1b, b'A']));
    }

    // -- Simple special keys --------------------------------------------------

    #[test]
    fn enter() {
        assert_eq!(
            encode_key(&key_event(Key::Enter, Modifiers::empty())),
            Some(vec![0x0D])
        );
    }

    #[test]
    fn tab() {
        assert_eq!(
            encode_key(&key_event(Key::Tab, Modifiers::empty())),
            Some(vec![0x09])
        );
    }

    #[test]
    fn backspace() {
        assert_eq!(
            encode_key(&key_event(Key::Backspace, Modifiers::empty())),
            Some(vec![0x7F])
        );
    }

    #[test]
    fn escape() {
        assert_eq!(
            encode_key(&key_event(Key::Escape, Modifiers::empty())),
            Some(vec![0x1B])
        );
    }

    #[test]
    fn space() {
        assert_eq!(
            encode_key(&key_event(Key::Space, Modifiers::empty())),
            Some(vec![0x20])
        );
    }

    #[test]
    fn ctrl_space() {
        assert_eq!(
            encode_key(&key_event(Key::Space, Modifiers::CTRL)),
            Some(vec![0x00])
        );
    }

    // -- Arrow keys -----------------------------------------------------------

    #[test]
    fn arrow_up() {
        assert_eq!(
            encode_key(&key_event(Key::ArrowUp, Modifiers::empty())),
            Some(vec![0x1b, b'[', b'A'])
        );
    }

    #[test]
    fn arrow_down() {
        assert_eq!(
            encode_key(&key_event(Key::ArrowDown, Modifiers::empty())),
            Some(vec![0x1b, b'[', b'B'])
        );
    }

    #[test]
    fn ctrl_arrow_right() {
        let result = encode_key(&key_event(Key::ArrowRight, Modifiers::CTRL));
        // Ctrl modifier = 5, so \x1b[1;5C
        assert_eq!(result, Some(b"\x1b[1;5C".to_vec()));
    }

    #[test]
    fn shift_arrow_left() {
        let result = encode_key(&key_event(Key::ArrowLeft, Modifiers::SHIFT));
        // Shift modifier = 2, so \x1b[1;2D
        assert_eq!(result, Some(b"\x1b[1;2D".to_vec()));
    }

    // -- Navigation keys ------------------------------------------------------

    #[test]
    fn home() {
        assert_eq!(
            encode_key(&key_event(Key::Home, Modifiers::empty())),
            Some(vec![0x1b, b'[', b'H'])
        );
    }

    #[test]
    fn end() {
        assert_eq!(
            encode_key(&key_event(Key::End, Modifiers::empty())),
            Some(vec![0x1b, b'[', b'F'])
        );
    }

    #[test]
    fn page_up() {
        assert_eq!(
            encode_key(&key_event(Key::PageUp, Modifiers::empty())),
            Some(b"\x1b[5~".to_vec())
        );
    }

    #[test]
    fn page_down() {
        assert_eq!(
            encode_key(&key_event(Key::PageDown, Modifiers::empty())),
            Some(b"\x1b[6~".to_vec())
        );
    }

    #[test]
    fn delete() {
        assert_eq!(
            encode_key(&key_event(Key::Delete, Modifiers::empty())),
            Some(b"\x1b[3~".to_vec())
        );
    }

    #[test]
    fn ctrl_delete() {
        let result = encode_key(&key_event(Key::Delete, Modifiers::CTRL));
        assert_eq!(result, Some(b"\x1b[3;5~".to_vec()));
    }

    // -- Function keys --------------------------------------------------------

    #[test]
    fn f1() {
        assert_eq!(
            encode_key(&key_event(Key::F(1), Modifiers::empty())),
            Some(vec![0x1b, b'O', b'P'])
        );
    }

    #[test]
    fn f4() {
        assert_eq!(
            encode_key(&key_event(Key::F(4), Modifiers::empty())),
            Some(vec![0x1b, b'O', b'S'])
        );
    }

    #[test]
    fn f5() {
        assert_eq!(
            encode_key(&key_event(Key::F(5), Modifiers::empty())),
            Some(b"\x1b[15~".to_vec())
        );
    }

    #[test]
    fn f12() {
        assert_eq!(
            encode_key(&key_event(Key::F(12), Modifiers::empty())),
            Some(b"\x1b[24~".to_vec())
        );
    }

    #[test]
    fn f13_returns_none() {
        assert!(encode_key(&key_event(Key::F(13), Modifiers::empty())).is_none());
    }

    #[test]
    fn ctrl_f1() {
        let result = encode_key(&key_event(Key::F(1), Modifiers::CTRL));
        // Modified F1: \x1b[1;5P
        assert_eq!(result, Some(b"\x1b[1;5P".to_vec()));
    }

    // -- Unknown key ----------------------------------------------------------

    #[test]
    fn unknown_key_returns_none() {
        assert!(encode_key(&key_event(Key::Unknown, Modifiers::empty())).is_none());
    }

    // -- xterm_modifier -------------------------------------------------------

    #[test]
    fn modifier_none() {
        assert_eq!(xterm_modifier(false, false, false), 0);
    }

    #[test]
    fn modifier_shift() {
        assert_eq!(xterm_modifier(true, false, false), 2);
    }

    #[test]
    fn modifier_alt() {
        assert_eq!(xterm_modifier(false, true, false), 3);
    }

    #[test]
    fn modifier_ctrl() {
        assert_eq!(xterm_modifier(false, false, true), 5);
    }

    #[test]
    fn modifier_shift_alt() {
        assert_eq!(xterm_modifier(true, true, false), 4);
    }

    #[test]
    fn modifier_shift_ctrl() {
        assert_eq!(xterm_modifier(true, false, true), 6);
    }

    #[test]
    fn modifier_alt_ctrl() {
        assert_eq!(xterm_modifier(false, true, true), 7);
    }

    #[test]
    fn modifier_all() {
        assert_eq!(xterm_modifier(true, true, true), 8);
    }

    // -- Modified F5-F12 keys (with Shift, Alt, or Ctrl) ----------------------

    #[test]
    fn shift_f5() {
        let result = encode_key(&key_event(Key::F(5), Modifiers::SHIFT));
        // F5 code=15, Shift modifier=2: \x1b[15;2~
        assert_eq!(result, Some(b"\x1b[15;2~".to_vec()));
    }

    #[test]
    fn alt_f8() {
        let result = encode_key(&key_event(Key::F(8), Modifiers::ALT));
        // F8 code=19, Alt modifier=3: \x1b[19;3~
        assert_eq!(result, Some(b"\x1b[19;3~".to_vec()));
    }

    #[test]
    fn ctrl_f12() {
        let result = encode_key(&key_event(Key::F(12), Modifiers::CTRL));
        // F12 code=24, Ctrl modifier=5: \x1b[24;5~
        assert_eq!(result, Some(b"\x1b[24;5~".to_vec()));
    }

    #[test]
    fn shift_ctrl_f6() {
        let result = encode_key(&key_event(Key::F(6), Modifiers::SHIFT | Modifiers::CTRL));
        // F6 code=17, Shift+Ctrl modifier=6: \x1b[17;6~
        assert_eq!(result, Some(b"\x1b[17;6~".to_vec()));
    }

    #[test]
    fn f7_unmodified() {
        let result = encode_key(&key_event(Key::F(7), Modifiers::empty()));
        // F7 code=18: \x1b[18~
        assert_eq!(result, Some(b"\x1b[18~".to_vec()));
    }

    #[test]
    fn f9_unmodified() {
        let result = encode_key(&key_event(Key::F(9), Modifiers::empty()));
        // F9 code=20: \x1b[20~
        assert_eq!(result, Some(b"\x1b[20~".to_vec()));
    }

    #[test]
    fn f10_unmodified() {
        let result = encode_key(&key_event(Key::F(10), Modifiers::empty()));
        // F10 code=21: \x1b[21~
        assert_eq!(result, Some(b"\x1b[21~".to_vec()));
    }

    #[test]
    fn f11_unmodified() {
        let result = encode_key(&key_event(Key::F(11), Modifiers::empty()));
        // F11 code=23: \x1b[23~
        assert_eq!(result, Some(b"\x1b[23~".to_vec()));
    }

    // -- Modified tilde sequences (PageUp, PageDown, Delete) ------------------

    #[test]
    fn shift_page_up() {
        let result = encode_key(&key_event(Key::PageUp, Modifiers::SHIFT));
        // PageUp num=5, Shift modifier=2: \x1b[5;2~
        assert_eq!(result, Some(b"\x1b[5;2~".to_vec()));
    }

    #[test]
    fn alt_delete() {
        let result = encode_key(&key_event(Key::Delete, Modifiers::ALT));
        // Delete num=3, Alt modifier=3: \x1b[3;3~
        assert_eq!(result, Some(b"\x1b[3;3~".to_vec()));
    }

    #[test]
    fn shift_page_down() {
        let result = encode_key(&key_event(Key::PageDown, Modifiers::SHIFT));
        // PageDown num=6, Shift modifier=2: \x1b[6;2~
        assert_eq!(result, Some(b"\x1b[6;2~".to_vec()));
    }

    // -- Modified Home/End keys -----------------------------------------------

    #[test]
    fn shift_home() {
        let result = encode_key(&key_event(Key::Home, Modifiers::SHIFT));
        // Home letter=H, Shift modifier=2: \x1b[1;2H
        assert_eq!(result, Some(b"\x1b[1;2H".to_vec()));
    }

    #[test]
    fn ctrl_end() {
        let result = encode_key(&key_event(Key::End, Modifiers::CTRL));
        // End letter=F, Ctrl modifier=5: \x1b[1;5F
        assert_eq!(result, Some(b"\x1b[1;5F".to_vec()));
    }

    // -- F(0) out-of-range returns None ---------------------------------------

    #[test]
    fn f0_returns_none() {
        assert!(encode_key(&key_event(Key::F(0), Modifiers::empty())).is_none());
    }
}

#[cfg(test)]
mod tests_insert_and_copy_regression {
    use super::tests::key_event;
    use super::*;

    // -- Insert key encoding (CSI-tilde with optional modifier) ----------------

    #[test]
    fn insert_plain() {
        let result = encode_key(&key_event(Key::Insert, Modifiers::empty()));
        // Insert: \x1b[2~
        assert_eq!(result, Some(b"\x1b[2~".to_vec()));
    }

    #[test]
    fn insert_with_shift() {
        let result = encode_key(&key_event(Key::Insert, Modifiers::SHIFT));
        // Insert code=2, Shift modifier=2: \x1b[2;2~
        assert_eq!(result, Some(b"\x1b[2;2~".to_vec()));
    }

    #[test]
    fn insert_with_ctrl() {
        let result = encode_key(&key_event(Key::Insert, Modifiers::CTRL));
        // Insert code=2, Ctrl modifier=5: \x1b[2;5~
        assert_eq!(result, Some(b"\x1b[2;5~".to_vec()));
    }

    #[test]
    fn insert_with_alt() {
        let result = encode_key(&key_event(Key::Insert, Modifiers::ALT));
        // Insert code=2, Alt modifier=3: \x1b[2;3~
        assert_eq!(result, Some(b"\x1b[2;3~".to_vec()));
    }

    #[test]
    fn insert_with_shift_ctrl() {
        let result = encode_key(&key_event(Key::Insert, Modifiers::SHIFT | Modifiers::CTRL));
        // Insert code=2, Shift+Ctrl modifier=6: \x1b[2;6~
        assert_eq!(result, Some(b"\x1b[2;6~".to_vec()));
    }

    // -- Regression: Ctrl+C and Ctrl+V must remain control codes ---------------
    // The selection/clipboard logic intercepts at the shortcut level
    // and is NOT part of encode_key. These must still reach the terminal
    // as raw control codes so shells receive them correctly.

    #[test]
    fn ctrl_c_is_interrupt_not_copy() {
        let result = encode_key(&key_event(Key::Char('c'), Modifiers::CTRL));
        // Ctrl+C MUST encode to SIGINT (0x03), not be a "copy" command.
        assert_eq!(result, Some(vec![0x03]));
    }

    #[test]
    fn ctrl_v_is_literal_input_not_paste() {
        let result = encode_key(&key_event(Key::Char('v'), Modifiers::CTRL));
        // Ctrl+V MUST encode to 0x16 (SYN/literal-next), not a "paste" command.
        assert_eq!(result, Some(vec![0x16]));
    }
}
