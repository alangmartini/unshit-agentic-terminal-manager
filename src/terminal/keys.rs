//! Map framework `Key` types to terminal escape sequences.
//!
//! Translates `KeyboardEvent` values from the unshit event system into the
//! byte sequences a PTY/shell expects. Supports plain characters, modifier
//! combinations (Ctrl, Alt, Shift), arrow keys, function keys, and common
//! special keys (Home, End, Page Up/Down, Delete, etc.).

use unshit::core::event::{Key, KeyboardEvent, KeyEventKind, Modifiers};

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
    if shift { m += 1; }
    if alt { m += 2; }
    if ctrl { m += 4; }
    if m == 1 { 0 } else { m }
}
