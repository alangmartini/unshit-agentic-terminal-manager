//! Pure multi-line text buffer for the built-in file editor.
//!
//! The buffer is deliberately standalone: it owns only text and line
//! structure, with no framework, state, or I/O imports, so every editing
//! behavior can be unit-tested without scaffolding. Storage is one
//! `String` per logical line with `\n`/`\r\n` normalized away at load
//! time (`LineEnding` remembers the original flavor for save).

/// Line-ending flavor detected at load time and preserved on save.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
        }
    }

    /// Detect the dominant line ending of `text`. Any `\r\n` marks the
    /// file as CRLF; otherwise LF. Files without newlines default to LF.
    pub fn detect(text: &str) -> Self {
        if text.contains("\r\n") {
            LineEnding::CrLf
        } else {
            LineEnding::Lf
        }
    }
}

/// Multi-line text buffer. Lines never contain `\n` or `\r`.
///
/// Invariant: `lines` is never empty — an empty document is one empty
/// line, matching how every editor models the cursor resting position.
#[derive(Clone, Debug, PartialEq)]
pub struct EditorBuffer {
    lines: Vec<String>,
}

impl EditorBuffer {
    /// Build a buffer from normalized text (no `\r`). `"a\n"` produces
    /// `["a", ""]` so a trailing newline round-trips through `to_text`.
    pub fn from_text(text: &str) -> Self {
        let lines: Vec<String> = text.split('\n').map(|l| l.to_string()).collect();
        debug_assert!(!lines.is_empty(), "str::split always yields one item");
        Self { lines }
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn line(&self, idx: usize) -> Option<&str> {
        self.lines.get(idx).map(|s| s.as_str())
    }

    /// Reassemble the document with `\n` separators (the normalized
    /// internal form). Callers re-apply the stored `LineEnding` on save.
    pub fn to_text(&self) -> String {
        self.lines.join("\n")
    }

    /// Longest line length in characters. Used to clamp horizontal scroll.
    pub fn max_line_chars(&self) -> usize {
        self.lines
            .iter()
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_is_one_empty_line() {
        let b = EditorBuffer::from_text("");
        assert_eq!(b.line_count(), 1);
        assert_eq!(b.line(0), Some(""));
        assert_eq!(b.to_text(), "");
    }

    #[test]
    fn trailing_newline_round_trips() {
        let b = EditorBuffer::from_text("a\n");
        assert_eq!(b.line_count(), 2);
        assert_eq!(b.line(0), Some("a"));
        assert_eq!(b.line(1), Some(""));
        assert_eq!(b.to_text(), "a\n");
    }

    #[test]
    fn multi_line_round_trips() {
        let text = "fn main() {\n    println!(\"héllo\");\n}";
        let b = EditorBuffer::from_text(text);
        assert_eq!(b.line_count(), 3);
        assert_eq!(b.to_text(), text);
    }

    #[test]
    fn line_ending_detection() {
        assert_eq!(LineEnding::detect("a\r\nb"), LineEnding::CrLf);
        assert_eq!(LineEnding::detect("a\nb"), LineEnding::Lf);
        assert_eq!(LineEnding::detect("plain"), LineEnding::Lf);
    }

    #[test]
    fn max_line_chars_counts_chars_not_bytes() {
        let b = EditorBuffer::from_text("ééé\nab");
        assert_eq!(b.max_line_chars(), 3);
    }
}
