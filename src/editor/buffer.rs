//! Pure multi-line text buffer for the built-in file editor.
//!
//! The buffer is deliberately standalone: it owns text, cursor,
//! selection, and (in a later slice) undo history, with no framework,
//! state, or I/O imports, so every editing behavior can be unit-tested
//! without scaffolding. Storage is one `String` per logical line with
//! `\n`/`\r\n` normalized away at load time (`LineEnding` remembers the
//! original flavor for save).
//!
//! Cursor columns are byte offsets clamped to `char` boundaries — the
//! same convention as the framework's single-line `InputState` — and
//! word boundaries mirror `unshit-core/src/input.rs` (same-class runs of
//! word chars `[alphanumeric_]` vs punctuation, whitespace skipped).

/// Spaces inserted for a Tab keypress (MVP: no tab characters).
pub const TAB_SPACES: usize = 4;

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

/// A cursor or selection endpoint: `line` index plus byte offset `col`
/// within that line (always on a char boundary).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    pub line: usize,
    pub col: usize,
}

/// Viewport repaint hint returned by buffer operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Damage {
    /// Nothing textual changed (pure cursor/selection movement).
    None,
    /// A single line's content changed.
    Line(usize),
    /// Line structure changed from this line downward (split/join/paste).
    From(usize),
}

impl Damage {
    /// Union of two damage hints.
    pub fn merge(self, other: Damage) -> Damage {
        use Damage::*;
        match (self, other) {
            (None, d) | (d, None) => d,
            (Line(a), Line(b)) if a == b => Line(a),
            (Line(a), Line(b)) => From(a.min(b)),
            (From(a), Line(b)) | (Line(b), From(a)) => From(a.min(b)),
            (From(a), From(b)) => From(a.min(b)),
        }
    }
}

/// Undo grouping category. `Typing` and `Deleting` runs coalesce into
/// one undo step (VS Code granularity); `Other` edits (paste, newline,
/// word deletes, selection replaces… ) get their own group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GroupKind {
    Typing,
    Deleting,
    Other,
}

/// One recorded range replacement: `old_text` at `start` became
/// `new_text`. Inverse application order restores either direction.
#[derive(Clone, Debug, PartialEq)]
struct EditRecord {
    start: Position,
    old_text: String,
    new_text: String,
    cursor_before: Position,
    anchor_before: Option<Position>,
    cursor_after: Position,
}

/// A single undo step. `id` is monotonically unique per buffer so the
/// pane can compare "top of undo stack" against the last-saved state
/// even across undo/redo/edit sequences.
#[derive(Clone, Debug, PartialEq)]
struct UndoGroup {
    id: u64,
    kind: GroupKind,
    records: Vec<EditRecord>,
}

/// Position of the end of `text` inserted at `start`.
fn end_of_text(start: Position, text: &str) -> Position {
    match text.rsplit_once('\n') {
        None => Position {
            line: start.line,
            col: start.col + text.len(),
        },
        Some((head, last)) => Position {
            line: start.line + head.matches('\n').count() + 1,
            col: last.len(),
        },
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Byte offset of the previous word boundary in `s` (mirrors the
/// framework input engine): skip trailing whitespace, then the
/// same-class run left of the cursor.
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

/// Byte offset of the next word boundary right of `pos`: skip the
/// current same-class run, then any whitespace after it.
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

/// Multi-line text buffer with cursor and selection. Lines never
/// contain `\n` or `\r`.
///
/// Invariant: `lines` is never empty — an empty document is one empty
/// line, matching how every editor models the cursor resting position.
#[derive(Clone, Debug, PartialEq)]
pub struct EditorBuffer {
    lines: Vec<String>,
    cursor: Position,
    /// Selection anchor; `Some` while a selection is active. The
    /// selection is the ordered span between anchor and cursor.
    anchor: Option<Position>,
    /// Desired visual column (in chars) preserved across vertical
    /// movement over short lines. Cleared by any horizontal move/edit.
    sticky_chars: Option<usize>,
    undo: Vec<UndoGroup>,
    redo: Vec<UndoGroup>,
    next_group_id: u64,
    /// True while the latest undo group may still absorb contiguous
    /// same-kind edits. Cursor moves and undo/redo close the group.
    group_open: bool,
}

impl EditorBuffer {
    /// Build a buffer from normalized text (no `\r`). `"a\n"` produces
    /// `["a", ""]` so a trailing newline round-trips through `to_text`.
    pub fn from_text(text: &str) -> Self {
        let lines: Vec<String> = text.split('\n').map(|l| l.to_string()).collect();
        debug_assert!(!lines.is_empty(), "str::split always yields one item");
        Self {
            lines,
            cursor: Position::default(),
            anchor: None,
            sticky_chars: None,
            undo: Vec::new(),
            redo: Vec::new(),
            next_group_id: 1,
            group_open: false,
        }
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

    pub fn cursor(&self) -> Position {
        self.cursor
    }

    /// Ordered selection span, `None` when empty or inactive.
    pub fn selection(&self) -> Option<(Position, Position)> {
        let anchor = self.anchor?;
        if anchor == self.cursor {
            return None;
        }
        if anchor < self.cursor {
            Some((anchor, self.cursor))
        } else {
            Some((self.cursor, anchor))
        }
    }

    /// Text covered by the current selection (with `\n` separators).
    pub fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection()?;
        Some(self.text_range(start, end))
    }

    /// Extract the text between two ordered positions.
    pub fn text_range(&self, start: Position, end: Position) -> String {
        if start.line == end.line {
            return self.lines[start.line][start.col..end.col].to_string();
        }
        let mut out = String::new();
        out.push_str(&self.lines[start.line][start.col..]);
        for line in &self.lines[start.line + 1..end.line] {
            out.push('\n');
            out.push_str(line);
        }
        out.push('\n');
        out.push_str(&self.lines[end.line][..end.col]);
        out
    }

    // -- position helpers ---------------------------------------------------

    fn clamp_position(&self, pos: Position) -> Position {
        let line = pos.line.min(self.lines.len() - 1);
        let col = self.snap_to_char_boundary(line, pos.col.min(self.lines[line].len()));
        Position { line, col }
    }

    fn snap_to_char_boundary(&self, line: usize, col: usize) -> usize {
        let s = &self.lines[line];
        let mut c = col.min(s.len());
        while c > 0 && !s.is_char_boundary(c) {
            c -= 1;
        }
        c
    }

    /// Character (not byte) column of a position — the visual cell
    /// column in a monospace grid before horizontal scroll.
    pub fn char_col(&self, pos: Position) -> usize {
        self.lines[pos.line][..pos.col].chars().count()
    }

    /// Byte offset of the `chars`-th character of `line` (clamped to
    /// line end). Inverse of `char_col` for cell→byte mapping.
    pub fn col_for_char_index(&self, line: usize, chars: usize) -> usize {
        let s = &self.lines[line];
        s.char_indices()
            .nth(chars)
            .map(|(b, _)| b)
            .unwrap_or(s.len())
    }

    fn prev_char_col(&self, line: usize, col: usize) -> usize {
        self.lines[line][..col]
            .char_indices()
            .next_back()
            .map(|(b, _)| b)
            .unwrap_or(0)
    }

    fn next_char_col(&self, line: usize, col: usize) -> usize {
        let s = &self.lines[line];
        s[col..]
            .chars()
            .next()
            .map(|c| col + c.len_utf8())
            .unwrap_or(s.len())
    }

    pub fn doc_end(&self) -> Position {
        let line = self.lines.len() - 1;
        Position {
            line,
            col: self.lines[line].len(),
        }
    }

    // -- movement -----------------------------------------------------------

    /// Shared prelude for movement: with `extend` the anchor is pinned,
    /// without it any selection collapses. Returns `true` when a
    /// collapse consumed the keypress (plain Left/Right on a selection
    /// jump to the selection edge without further movement).
    fn begin_move(&mut self, extend: bool) {
        // Any cursor movement closes the open undo group (VS Code
        // granularity: a typing run interrupted by navigation undoes in
        // two steps).
        self.group_open = false;
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor);
            }
        } else {
            self.anchor = None;
        }
    }

    pub fn move_left(&mut self, extend: bool) {
        self.sticky_chars = None;
        if !extend {
            if let Some((start, _)) = self.selection() {
                self.cursor = start;
                self.anchor = None;
                return;
            }
        }
        self.begin_move(extend);
        if self.cursor.col > 0 {
            self.cursor.col = self.prev_char_col(self.cursor.line, self.cursor.col);
        } else if self.cursor.line > 0 {
            self.cursor.line -= 1;
            self.cursor.col = self.lines[self.cursor.line].len();
        }
    }

    pub fn move_right(&mut self, extend: bool) {
        self.sticky_chars = None;
        if !extend {
            if let Some((_, end)) = self.selection() {
                self.cursor = end;
                self.anchor = None;
                return;
            }
        }
        self.begin_move(extend);
        if self.cursor.col < self.lines[self.cursor.line].len() {
            self.cursor.col = self.next_char_col(self.cursor.line, self.cursor.col);
        } else if self.cursor.line + 1 < self.lines.len() {
            self.cursor.line += 1;
            self.cursor.col = 0;
        }
    }

    fn move_vertical(&mut self, delta: isize, extend: bool) {
        if !extend {
            if let Some((start, end)) = self.selection() {
                self.cursor = if delta < 0 { start } else { end };
            }
        }
        self.begin_move(extend);
        let sticky = self
            .sticky_chars
            .unwrap_or_else(|| self.char_col(self.cursor));
        self.sticky_chars = Some(sticky);
        let target_line = self
            .cursor
            .line
            .saturating_add_signed(delta)
            .min(self.lines.len() - 1);
        if target_line == self.cursor.line {
            // Hitting the document edge: VS Code snaps to line start/end.
            if delta < 0 {
                self.cursor.col = 0;
            } else {
                self.cursor.col = self.lines[self.cursor.line].len();
            }
            self.sticky_chars = None;
            return;
        }
        self.cursor.line = target_line;
        self.cursor.col = self.col_for_char_index(target_line, sticky);
    }

    pub fn move_up(&mut self, extend: bool) {
        self.move_vertical(-1, extend);
    }

    pub fn move_down(&mut self, extend: bool) {
        self.move_vertical(1, extend);
    }

    pub fn move_page(&mut self, delta_lines: isize, extend: bool) {
        self.move_vertical(delta_lines, extend);
    }

    pub fn move_home(&mut self, extend: bool) {
        self.sticky_chars = None;
        self.begin_move(extend);
        self.cursor.col = 0;
    }

    pub fn move_end(&mut self, extend: bool) {
        self.sticky_chars = None;
        self.begin_move(extend);
        self.cursor.col = self.lines[self.cursor.line].len();
    }

    pub fn move_doc_start(&mut self, extend: bool) {
        self.sticky_chars = None;
        self.begin_move(extend);
        self.cursor = Position::default();
    }

    pub fn move_doc_end(&mut self, extend: bool) {
        self.sticky_chars = None;
        self.begin_move(extend);
        self.cursor = self.doc_end();
    }

    pub fn move_word_left(&mut self, extend: bool) {
        self.sticky_chars = None;
        self.begin_move(extend);
        if self.cursor.col == 0 {
            if self.cursor.line > 0 {
                self.cursor.line -= 1;
                self.cursor.col = self.lines[self.cursor.line].len();
            }
            return;
        }
        self.cursor.col = prev_word_boundary(&self.lines[self.cursor.line], self.cursor.col);
    }

    pub fn move_word_right(&mut self, extend: bool) {
        self.sticky_chars = None;
        self.begin_move(extend);
        let line_len = self.lines[self.cursor.line].len();
        if self.cursor.col >= line_len {
            if self.cursor.line + 1 < self.lines.len() {
                self.cursor.line += 1;
                self.cursor.col = 0;
            }
            return;
        }
        self.cursor.col = next_word_boundary(&self.lines[self.cursor.line], self.cursor.col);
    }

    /// Place the cursor at `pos` (clamped), optionally extending the
    /// selection (shift-click).
    pub fn set_cursor(&mut self, pos: Position, extend: bool) {
        self.sticky_chars = None;
        self.begin_move(extend);
        self.cursor = self.clamp_position(pos);
    }

    pub fn select_all(&mut self) {
        self.sticky_chars = None;
        self.anchor = Some(Position::default());
        self.cursor = self.doc_end();
    }

    /// Select the same-class word run under `pos` (double-click).
    pub fn select_word_at(&mut self, pos: Position) {
        self.sticky_chars = None;
        let pos = self.clamp_position(pos);
        let line = &self.lines[pos.line];
        if line.is_empty() {
            self.anchor = None;
            self.cursor = pos;
            return;
        }
        let col = if pos.col >= line.len() {
            self.prev_char_col(pos.line, line.len())
        } else {
            pos.col
        };
        let target = line[col..].chars().next().unwrap_or(' ');
        let class_of = |c: char| {
            if c.is_whitespace() {
                2u8
            } else if is_word_char(c) {
                0
            } else {
                1
            }
        };
        let class = class_of(target);
        let mut start = col;
        while start > 0 {
            let prev = self.prev_char_col(pos.line, start);
            let c = line[prev..].chars().next().unwrap();
            if class_of(c) != class {
                break;
            }
            start = prev;
        }
        let mut end = col;
        while end < line.len() {
            let c = line[end..].chars().next().unwrap();
            if class_of(c) != class {
                break;
            }
            end += c.len_utf8();
        }
        self.anchor = Some(Position {
            line: pos.line,
            col: start,
        });
        self.cursor = Position {
            line: pos.line,
            col: end,
        };
    }

    // -- editing ------------------------------------------------------------

    /// Splice `[start, end)` out and `text` in, without recording undo
    /// history. Cursor lands at the end of the inserted text. This is
    /// the single primitive every edit (and undo/redo replay) uses.
    fn raw_replace(&mut self, start: Position, end: Position, text: &str) -> Damage {
        let removal = if start == end {
            Damage::None
        } else if start.line == end.line {
            self.lines[start.line].replace_range(start.col..end.col, "");
            Damage::Line(start.line)
        } else {
            let tail = self.lines[end.line][end.col..].to_string();
            self.lines[start.line].truncate(start.col);
            self.lines[start.line].push_str(&tail);
            self.lines.drain(start.line + 1..=end.line);
            Damage::From(start.line)
        };
        let insertion = if text.is_empty() {
            Damage::None
        } else if !text.contains('\n') {
            self.lines[start.line].insert_str(start.col, text);
            Damage::Line(start.line)
        } else {
            let tail = self.lines[start.line][start.col..].to_string();
            self.lines[start.line].truncate(start.col);
            let mut parts = text.split('\n');
            self.lines[start.line].push_str(parts.next().unwrap_or_default());
            let mut last_line = start.line;
            for (insert_at, part) in (start.line + 1..).zip(parts) {
                self.lines.insert(insert_at, part.to_string());
                last_line = insert_at;
            }
            self.lines[last_line].push_str(&tail);
            Damage::From(start.line)
        };
        self.cursor = end_of_text(start, text);
        self.anchor = None;
        removal.merge(insertion)
    }

    /// Perform and record a range replacement, coalescing into the open
    /// undo group when the kind and adjacency allow it.
    fn record_replace(
        &mut self,
        start: Position,
        end: Position,
        text: &str,
        kind: GroupKind,
    ) -> Damage {
        let old_text = self.text_range(start, end);
        if old_text.is_empty() && text.is_empty() {
            return Damage::None;
        }
        let record = EditRecord {
            start,
            old_text,
            new_text: text.to_string(),
            cursor_before: self.cursor,
            anchor_before: self.anchor,
            cursor_after: end_of_text(start, text),
        };
        let damage = self.raw_replace(start, end, text);
        self.push_record(kind, record);
        damage
    }

    fn push_record(&mut self, kind: GroupKind, record: EditRecord) {
        self.redo.clear();
        if self.group_open && kind != GroupKind::Other {
            if let Some(group) = self.undo.last_mut() {
                if group.kind == kind {
                    if let Some(last) = group.records.last_mut() {
                        match kind {
                            // Typing run: appended right at the end of the
                            // previous insertion.
                            GroupKind::Typing
                                if record.old_text.is_empty()
                                    && record.start == end_of_text(last.start, &last.new_text) =>
                            {
                                last.new_text.push_str(&record.new_text);
                                last.cursor_after = record.cursor_after;
                                return;
                            }
                            // Backspace run: each delete ends where the
                            // previous one started.
                            GroupKind::Deleting
                                if record.new_text.is_empty()
                                    && end_of_text(record.start, &record.old_text)
                                        == last.start =>
                            {
                                last.old_text = format!("{}{}", record.old_text, last.old_text);
                                last.start = record.start;
                                last.cursor_after = record.cursor_after;
                                return;
                            }
                            // Delete-forward run: repeated deletes at the
                            // same spot.
                            GroupKind::Deleting
                                if record.new_text.is_empty() && record.start == last.start =>
                            {
                                last.old_text.push_str(&record.old_text);
                                last.cursor_after = record.cursor_after;
                                return;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        let id = self.next_group_id;
        self.next_group_id += 1;
        self.undo.push(UndoGroup {
            id,
            kind,
            records: vec![record],
        });
        self.group_open = kind != GroupKind::Other;
    }

    /// Identity of the newest undo step (0 for a pristine buffer). The
    /// pane compares this against the id captured at save time to derive
    /// the dirty flag correctly across undo/redo.
    pub fn top_group_id(&self) -> u64 {
        self.undo.last().map(|g| g.id).unwrap_or(0)
    }

    /// Close the open undo group so the next edit starts a fresh step
    /// (called on save).
    pub fn break_undo_group(&mut self) {
        self.group_open = false;
    }

    /// Undo the newest edit group. Returns the repaint damage
    /// (`Damage::None` when there is nothing to undo).
    pub fn undo(&mut self) -> Damage {
        self.sticky_chars = None;
        self.group_open = false;
        let Some(group) = self.undo.pop() else {
            return Damage::None;
        };
        let mut damage = Damage::None;
        for rec in group.records.iter().rev() {
            let end = end_of_text(rec.start, &rec.new_text);
            damage = damage.merge(self.raw_replace(rec.start, end, &rec.old_text));
        }
        let first = group.records.first().expect("groups are never empty");
        self.cursor = first.cursor_before;
        self.anchor = first.anchor_before;
        self.redo.push(group);
        damage
    }

    /// Re-apply the newest undone group.
    pub fn redo(&mut self) -> Damage {
        self.sticky_chars = None;
        self.group_open = false;
        let Some(group) = self.redo.pop() else {
            return Damage::None;
        };
        let mut damage = Damage::None;
        for rec in group.records.iter() {
            let end = end_of_text(rec.start, &rec.old_text);
            damage = damage.merge(self.raw_replace(rec.start, end, &rec.new_text));
        }
        let last = group.records.last().expect("groups are never empty");
        self.cursor = last.cursor_after;
        self.anchor = None;
        self.undo.push(group);
        damage
    }

    /// Delete the active selection, if any.
    pub fn delete_selection(&mut self) -> Damage {
        self.sticky_chars = None;
        match self.selection() {
            Some((start, end)) => self.record_replace(start, end, "", GroupKind::Other),
            None => Damage::None,
        }
    }

    fn insert_with_kind(&mut self, text: &str, kind: GroupKind) -> Damage {
        self.sticky_chars = None;
        let normalized;
        let text = if text.contains('\r') {
            normalized = text.replace("\r\n", "\n").replace('\r', "\n");
            normalized.as_str()
        } else {
            text
        };
        let (start, end) = self.selection().unwrap_or((self.cursor, self.cursor));
        self.record_replace(start, end, text, kind)
    }

    /// Insert text at the cursor (replacing any selection) as its own
    /// undo step. `text` may contain `\n` (multi-line paste); `\r` is
    /// normalized away.
    pub fn insert_str(&mut self, text: &str) -> Damage {
        self.insert_with_kind(text, GroupKind::Other)
    }

    /// Insert keystroke text; consecutive typed inserts coalesce into
    /// one undo step.
    pub fn insert_typed(&mut self, text: &str) -> Damage {
        self.insert_with_kind(text, GroupKind::Typing)
    }

    pub fn insert_char(&mut self, ch: char) -> Damage {
        let mut buf = [0u8; 4];
        self.insert_typed(ch.encode_utf8(&mut buf))
    }

    /// Split the current line at the cursor (Enter). Its own undo step.
    pub fn insert_newline(&mut self) -> Damage {
        self.insert_str("\n")
    }

    /// Backspace: delete selection, or the char (Ctrl: word) before the
    /// cursor, joining lines at column 0.
    pub fn backspace(&mut self, word: bool) -> Damage {
        self.sticky_chars = None;
        if self.selection().is_some() {
            return self.delete_selection();
        }
        let Position { line, col } = self.cursor;
        let (start, kind) = if col == 0 {
            if line == 0 {
                return Damage::None;
            }
            (
                Position {
                    line: line - 1,
                    col: self.lines[line - 1].len(),
                },
                GroupKind::Other,
            )
        } else if word {
            (
                Position {
                    line,
                    col: prev_word_boundary(&self.lines[line], col),
                },
                GroupKind::Other,
            )
        } else {
            (
                Position {
                    line,
                    col: self.prev_char_col(line, col),
                },
                GroupKind::Deleting,
            )
        };
        self.record_replace(start, self.cursor, "", kind)
    }

    /// Delete: delete selection, or the char (Ctrl: word) after the
    /// cursor, joining lines at end of line.
    pub fn delete_forward(&mut self, word: bool) -> Damage {
        self.sticky_chars = None;
        if self.selection().is_some() {
            return self.delete_selection();
        }
        let Position { line, col } = self.cursor;
        let line_len = self.lines[line].len();
        let (end, kind) = if col >= line_len {
            if line + 1 >= self.lines.len() {
                return Damage::None;
            }
            (
                Position {
                    line: line + 1,
                    col: 0,
                },
                GroupKind::Other,
            )
        } else if word {
            (
                Position {
                    line,
                    col: next_word_boundary(&self.lines[line], col),
                },
                GroupKind::Other,
            )
        } else {
            (
                Position {
                    line,
                    col: self.next_char_col(line, col),
                },
                GroupKind::Deleting,
            )
        };
        self.record_replace(self.cursor, end, "", kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(text: &str) -> EditorBuffer {
        EditorBuffer::from_text(text)
    }

    fn at(line: usize, col: usize) -> Position {
        Position { line, col }
    }

    // -- construction / round-trip -----------------------------------------

    #[test]
    fn empty_text_is_one_empty_line() {
        let b = buf("");
        assert_eq!(b.line_count(), 1);
        assert_eq!(b.line(0), Some(""));
        assert_eq!(b.to_text(), "");
    }

    #[test]
    fn trailing_newline_round_trips() {
        let b = buf("a\n");
        assert_eq!(b.line_count(), 2);
        assert_eq!(b.to_text(), "a\n");
    }

    #[test]
    fn multi_line_round_trips() {
        let text = "fn main() {\n    println!(\"héllo\");\n}";
        assert_eq!(buf(text).to_text(), text);
    }

    #[test]
    fn line_ending_detection() {
        assert_eq!(LineEnding::detect("a\r\nb"), LineEnding::CrLf);
        assert_eq!(LineEnding::detect("a\nb"), LineEnding::Lf);
        assert_eq!(LineEnding::detect("plain"), LineEnding::Lf);
    }

    #[test]
    fn max_line_chars_counts_chars_not_bytes() {
        assert_eq!(buf("ééé\nab").max_line_chars(), 3);
    }

    // -- insertion ----------------------------------------------------------

    #[test]
    fn insert_char_at_start_middle_end() {
        let mut b = buf("ab");
        assert_eq!(b.insert_char('X'), Damage::Line(0));
        assert_eq!(b.to_text(), "Xab");
        b.set_cursor(at(0, 2), false);
        b.insert_char('Y');
        assert_eq!(b.to_text(), "XaYb");
        b.move_end(false);
        b.insert_char('Z');
        assert_eq!(b.to_text(), "XaYbZ");
        assert_eq!(b.cursor(), at(0, 5));
    }

    #[test]
    fn insert_multibyte_char_advances_by_utf8_len() {
        let mut b = buf("");
        b.insert_char('é');
        assert_eq!(b.cursor(), at(0, 2));
        b.insert_char('!');
        assert_eq!(b.to_text(), "é!");
    }

    #[test]
    fn insert_newline_splits_line() {
        let mut b = buf("hello world");
        b.set_cursor(at(0, 5), false);
        assert_eq!(b.insert_newline(), Damage::From(0));
        assert_eq!(b.to_text(), "hello\n world");
        assert_eq!(b.cursor(), at(1, 0));
    }

    #[test]
    fn insert_newline_at_line_end_creates_empty_line() {
        let mut b = buf("abc");
        b.move_end(false);
        b.insert_newline();
        assert_eq!(b.to_text(), "abc\n");
        assert_eq!(b.cursor(), at(1, 0));
    }

    #[test]
    fn insert_multiline_paste() {
        let mut b = buf("head tail");
        b.set_cursor(at(0, 4), false);
        assert_eq!(b.insert_str("A\nB\nC"), Damage::From(0));
        assert_eq!(b.to_text(), "headA\nB\nC tail");
        assert_eq!(b.cursor(), at(2, 1));
    }

    #[test]
    fn insert_str_normalizes_crlf() {
        let mut b = buf("");
        b.insert_str("a\r\nb\rc");
        assert_eq!(b.to_text(), "a\nb\nc");
    }

    #[test]
    fn typing_replaces_selection() {
        let mut b = buf("hello world");
        b.set_cursor(at(0, 0), false);
        b.move_word_right(true); // select "hello "
        b.insert_char('X');
        assert_eq!(b.to_text(), "Xworld");
        assert_eq!(b.selection(), None);
    }

    // -- deletion -----------------------------------------------------------

    #[test]
    fn backspace_deletes_prev_char() {
        let mut b = buf("abé");
        b.move_doc_end(false);
        assert_eq!(b.backspace(false), Damage::Line(0));
        assert_eq!(b.to_text(), "ab");
    }

    #[test]
    fn backspace_at_line_start_joins_lines() {
        let mut b = buf("ab\ncd");
        b.set_cursor(at(1, 0), false);
        assert_eq!(b.backspace(false), Damage::From(0));
        assert_eq!(b.to_text(), "abcd");
        assert_eq!(b.cursor(), at(0, 2));
    }

    #[test]
    fn backspace_at_doc_start_is_noop() {
        let mut b = buf("ab");
        assert_eq!(b.backspace(false), Damage::None);
        assert_eq!(b.to_text(), "ab");
    }

    #[test]
    fn delete_forward_and_join() {
        let mut b = buf("ab\ncd");
        b.set_cursor(at(0, 2), false);
        assert_eq!(b.delete_forward(false), Damage::From(0));
        assert_eq!(b.to_text(), "abcd");
        b.set_cursor(at(0, 0), false);
        b.delete_forward(false);
        assert_eq!(b.to_text(), "bcd");
    }

    #[test]
    fn delete_forward_at_doc_end_is_noop() {
        let mut b = buf("ab");
        b.move_doc_end(false);
        assert_eq!(b.delete_forward(false), Damage::None);
    }

    #[test]
    fn word_backspace_deletes_word_run() {
        let mut b = buf("foo bar_baz");
        b.move_doc_end(false);
        b.backspace(true);
        assert_eq!(b.to_text(), "foo ");
        b.backspace(true);
        assert_eq!(b.to_text(), "");
    }

    #[test]
    fn word_delete_forward() {
        let mut b = buf("foo bar");
        b.set_cursor(at(0, 0), false);
        b.delete_forward(true);
        assert_eq!(b.to_text(), "bar");
    }

    #[test]
    fn delete_selection_spanning_lines() {
        let mut b = buf("one\ntwo\nthree");
        b.set_cursor(at(0, 1), false);
        b.set_cursor(at(2, 2), true);
        assert_eq!(b.delete_selection(), Damage::From(0));
        assert_eq!(b.to_text(), "oree");
        assert_eq!(b.cursor(), at(0, 1));
    }

    // -- movement -----------------------------------------------------------

    #[test]
    fn left_right_cross_line_boundaries() {
        let mut b = buf("ab\ncd");
        b.set_cursor(at(1, 0), false);
        b.move_left(false);
        assert_eq!(b.cursor(), at(0, 2));
        b.move_right(false);
        assert_eq!(b.cursor(), at(1, 0));
    }

    #[test]
    fn left_right_are_char_wise_not_byte_wise() {
        let mut b = buf("aé b");
        b.move_doc_end(false);
        b.move_left(false);
        b.move_left(false);
        assert_eq!(b.cursor(), at(0, 3)); // after 'é' (2 bytes)
        b.move_left(false);
        assert_eq!(b.cursor(), at(0, 1));
    }

    #[test]
    fn plain_arrow_collapses_selection_to_edge() {
        let mut b = buf("abcdef");
        b.set_cursor(at(0, 1), false);
        b.set_cursor(at(0, 4), true);
        b.move_left(false);
        assert_eq!(b.cursor(), at(0, 1));
        assert_eq!(b.selection(), None);

        b.set_cursor(at(0, 1), false);
        b.set_cursor(at(0, 4), true);
        b.move_right(false);
        assert_eq!(b.cursor(), at(0, 4));
    }

    #[test]
    fn sticky_column_survives_short_lines() {
        let mut b = buf("longline\nab\nlongerline");
        b.set_cursor(at(0, 6), false);
        b.move_down(false);
        assert_eq!(b.cursor(), at(1, 2)); // clamped to "ab"
        b.move_down(false);
        assert_eq!(b.cursor(), at(2, 6)); // sticky restored
    }

    #[test]
    fn sticky_column_counts_chars_for_multibyte_lines() {
        let mut b = buf("ééééé\nabcde");
        b.set_cursor(at(0, 6), false); // 3 chars in
        b.move_down(false);
        assert_eq!(b.cursor(), at(1, 3));
    }

    #[test]
    fn vertical_move_at_doc_edges_snaps_to_line_ends() {
        let mut b = buf("abc\ndef");
        b.set_cursor(at(0, 1), false);
        b.move_up(false);
        assert_eq!(b.cursor(), at(0, 0));
        b.set_cursor(at(1, 1), false);
        b.move_down(false);
        assert_eq!(b.cursor(), at(1, 3));
    }

    #[test]
    fn home_end_doc_start_doc_end() {
        let mut b = buf("abc\ndef");
        b.set_cursor(at(1, 2), false);
        b.move_home(false);
        assert_eq!(b.cursor(), at(1, 0));
        b.move_end(false);
        assert_eq!(b.cursor(), at(1, 3));
        b.move_doc_start(false);
        assert_eq!(b.cursor(), at(0, 0));
        b.move_doc_end(false);
        assert_eq!(b.cursor(), at(1, 3));
    }

    #[test]
    fn word_movement_crosses_lines() {
        let mut b = buf("foo bar\nbaz");
        b.set_cursor(at(0, 0), false);
        b.move_word_right(false);
        assert_eq!(b.cursor(), at(0, 4));
        b.move_word_right(false);
        assert_eq!(b.cursor(), at(0, 7));
        b.move_word_right(false);
        assert_eq!(b.cursor(), at(1, 0));
        b.move_word_left(false);
        assert_eq!(b.cursor(), at(0, 7));
        b.move_word_left(false);
        assert_eq!(b.cursor(), at(0, 4));
    }

    #[test]
    fn page_move_uses_sticky_column() {
        let text: Vec<String> = (0..50).map(|i| format!("line {}", i)).collect();
        let mut b = buf(&text.join("\n"));
        b.set_cursor(at(0, 5), false);
        b.move_page(10, false);
        assert_eq!(b.cursor(), at(10, 5));
        b.move_page(-100, false);
        assert_eq!(b.cursor().line, 0);
    }

    // -- selection ----------------------------------------------------------

    #[test]
    fn shift_extends_in_all_directions() {
        let mut b = buf("abc\ndef\nghi");
        b.set_cursor(at(1, 1), false);
        b.move_right(true);
        assert_eq!(b.selection(), Some((at(1, 1), at(1, 2))));
        b.move_down(true);
        assert_eq!(b.selection(), Some((at(1, 1), at(2, 2))));
        b.move_up(true);
        b.move_up(true);
        assert_eq!(b.selection(), Some((at(0, 2), at(1, 1))));
        b.move_left(true);
        assert_eq!(b.selection(), Some((at(0, 1), at(1, 1))));
    }

    #[test]
    fn movement_without_shift_clears_selection() {
        let mut b = buf("abc");
        b.select_all();
        assert!(b.selection().is_some());
        b.move_up(false);
        assert_eq!(b.selection(), None);
    }

    #[test]
    fn select_all_spans_document() {
        let mut b = buf("ab\ncd");
        b.select_all();
        assert_eq!(b.selection(), Some((at(0, 0), at(1, 2))));
        assert_eq!(b.selected_text().as_deref(), Some("ab\ncd"));
    }

    #[test]
    fn selected_text_multiline_slice() {
        let mut b = buf("one\ntwo\nthree");
        b.set_cursor(at(0, 1), false);
        b.set_cursor(at(2, 2), true);
        assert_eq!(b.selected_text().as_deref(), Some("ne\ntwo\nth"));
    }

    #[test]
    fn select_word_at_picks_word_run() {
        let mut b = buf("foo bar_baz qux");
        b.select_word_at(at(0, 6));
        assert_eq!(b.selected_text().as_deref(), Some("bar_baz"));
        // Past end of line selects the trailing word.
        b.select_word_at(at(0, 99));
        assert_eq!(b.selected_text().as_deref(), Some("qux"));
    }

    #[test]
    fn empty_selection_is_none() {
        let mut b = buf("abc");
        b.set_cursor(at(0, 1), false);
        b.set_cursor(at(0, 1), true);
        assert_eq!(b.selection(), None);
    }

    #[test]
    fn set_cursor_clamps_and_snaps_to_char_boundary() {
        let mut b = buf("aé");
        b.set_cursor(at(9, 9), false);
        assert_eq!(b.cursor(), at(0, 3));
        b.set_cursor(at(0, 2), false); // inside 'é'
        assert_eq!(b.cursor(), at(0, 1));
    }

    // -- undo / redo --------------------------------------------------------

    #[test]
    fn typing_run_undoes_as_one_step() {
        let mut b = buf("base");
        for c in "hello".chars() {
            b.insert_char(c);
        }
        assert_eq!(b.to_text(), "hellobase");
        assert_ne!(b.undo(), Damage::None);
        assert_eq!(b.to_text(), "base");
        assert_eq!(b.cursor(), at(0, 0));
        // Nothing further to undo.
        assert_eq!(b.undo(), Damage::None);
    }

    #[test]
    fn movement_breaks_typing_group() {
        let mut b = buf("");
        b.insert_char('a');
        b.insert_char('b');
        b.move_left(false);
        b.move_right(false);
        b.insert_char('c');
        assert_eq!(b.to_text(), "abc");
        b.undo();
        assert_eq!(b.to_text(), "ab");
        b.undo();
        assert_eq!(b.to_text(), "");
    }

    #[test]
    fn backspace_run_undoes_as_one_step() {
        let mut b = buf("hello");
        b.move_doc_end(false);
        b.backspace(false);
        b.backspace(false);
        b.backspace(false);
        assert_eq!(b.to_text(), "he");
        b.undo();
        assert_eq!(b.to_text(), "hello");
        assert_eq!(b.cursor(), at(0, 5));
    }

    #[test]
    fn delete_forward_run_undoes_as_one_step() {
        let mut b = buf("hello");
        b.set_cursor(at(0, 0), false);
        b.delete_forward(false);
        b.delete_forward(false);
        assert_eq!(b.to_text(), "llo");
        b.undo();
        assert_eq!(b.to_text(), "hello");
    }

    #[test]
    fn redo_reapplies_and_new_edit_clears_redo() {
        let mut b = buf("");
        b.insert_char('a');
        b.move_left(false); // break group
        b.insert_char('b');
        b.undo();
        assert_eq!(b.to_text(), "a");
        assert_ne!(b.redo(), Damage::None);
        assert_eq!(b.to_text(), "ba");
        b.undo();
        b.insert_char('c');
        // Redo history is gone after a fresh edit.
        assert_eq!(b.redo(), Damage::None);
        assert_eq!(b.to_text(), "ca");
    }

    #[test]
    fn undo_restores_replaced_selection() {
        let mut b = buf("hello world");
        b.set_cursor(at(0, 0), false);
        b.move_word_right(true); // select "hello "
        b.insert_char('X');
        assert_eq!(b.to_text(), "Xworld");
        b.undo();
        assert_eq!(b.to_text(), "hello world");
        // Cursor and selection restored to the pre-edit state.
        assert_eq!(b.selection(), Some((at(0, 0), at(0, 6))));
    }

    #[test]
    fn undo_of_multiline_paste_removes_all_lines() {
        let mut b = buf("head tail");
        b.set_cursor(at(0, 4), false);
        b.insert_str("A\nB\nC");
        assert_eq!(b.to_text(), "headA\nB\nC tail");
        b.undo();
        assert_eq!(b.to_text(), "head tail");
        assert_eq!(b.cursor(), at(0, 4));
        b.redo();
        assert_eq!(b.to_text(), "headA\nB\nC tail");
        assert_eq!(b.cursor(), at(2, 1));
    }

    #[test]
    fn undo_of_line_join_restores_newline() {
        let mut b = buf("ab\ncd");
        b.set_cursor(at(1, 0), false);
        b.backspace(false);
        assert_eq!(b.to_text(), "abcd");
        b.undo();
        assert_eq!(b.to_text(), "ab\ncd");
        assert_eq!(b.cursor(), at(1, 0));
    }

    #[test]
    fn top_group_id_tracks_edit_identity_across_undo() {
        let mut b = buf("");
        assert_eq!(b.top_group_id(), 0);
        b.insert_char('a');
        let first = b.top_group_id();
        assert_ne!(first, 0);
        b.undo();
        assert_eq!(b.top_group_id(), 0);
        b.redo();
        assert_eq!(b.top_group_id(), first);
        // A fresh edit after undo gets a new identity, so "back to
        // saved depth" cannot be confused with "same content".
        b.undo();
        b.insert_char('b');
        assert_ne!(b.top_group_id(), first);
    }

    #[test]
    fn paste_is_its_own_undo_step_between_typing() {
        let mut b = buf("");
        b.insert_char('a');
        b.insert_str("PASTE");
        b.insert_char('b');
        assert_eq!(b.to_text(), "aPASTEb");
        b.undo();
        assert_eq!(b.to_text(), "aPASTE");
        b.undo();
        assert_eq!(b.to_text(), "a");
        b.undo();
        assert_eq!(b.to_text(), "");
    }

    // -- damage -------------------------------------------------------------

    #[test]
    fn damage_merge_prefers_widest() {
        use Damage::*;
        assert_eq!(None.merge(Line(3)), Line(3));
        assert_eq!(Line(3).merge(Line(3)), Line(3));
        assert_eq!(Line(3).merge(Line(5)), From(3));
        assert_eq!(From(4).merge(Line(2)), From(2));
        assert_eq!(From(4).merge(From(6)), From(4));
    }
}
