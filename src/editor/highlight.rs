//! Per-pane syntax state: turns a buffer line into coloured spans.
//!
//! The scanner in [`crate::syntax`] is line-at-a-time and carries one bit
//! of context between lines — whether the line starts inside a `/* … */`
//! block comment. Painting a viewport means starting at an arbitrary
//! line, so that bit has to be reconstructed from the top of the file.
//! This cache does it once and then incrementally: `states[n]` is the
//! block-comment flag at the start of line `n`, valid for the first
//! [`SyntaxCache::valid_len`] lines, extended lazily as the viewport
//! moves down and truncated at the first line an edit touched.
//!
//! Languages without block comments never need the walk at all — their
//! state is always `false` — which covers the shells, Python, YAML and
//! friends outright.

use crate::syntax::{tokenize_spans, Language, Span};

use super::buffer::EditorBuffer;

/// Block-comment state per line plus a scratch span buffer, so painting a
/// row allocates nothing.
pub struct SyntaxCache {
    lang: Language,
    /// `states[n]` = "line n starts inside a block comment". Only the
    /// first `valid_len` entries are trustworthy.
    states: Vec<bool>,
    valid_len: usize,
    scratch: Vec<Span>,
}

impl SyntaxCache {
    pub fn new(lang: Language) -> Self {
        Self {
            lang,
            states: Vec::new(),
            valid_len: 0,
            scratch: Vec::new(),
        }
    }

    pub fn language(&self) -> Language {
        self.lang
    }

    /// Highlighting is off for plain text: there is nothing to colour and
    /// the painter can skip the per-line work entirely.
    pub fn is_active(&self) -> bool {
        self.lang != Language::Plain
    }

    /// Drop cached state from `line` onward. Call on every buffer edit
    /// with the first damaged line: inserting `/*` changes the comment
    /// state of everything below it.
    pub fn invalidate_from(&mut self, line: usize) {
        self.valid_len = self.valid_len.min(line);
        self.states.truncate(self.valid_len);
    }

    /// Token spans covering `line` with no gaps, or an empty slice when
    /// highlighting is off or the line does not exist.
    ///
    /// Extending the cache to a line far below the last painted one
    /// re-scans every line in between (once). For a language with block
    /// comments that is the price of correctness; it is bounded by the
    /// 16 MiB open limit and is paid on jumps, not on typing.
    pub fn spans_for(&mut self, buffer: &EditorBuffer, line: usize) -> &[Span] {
        self.scratch.clear();
        if !self.is_active() || line >= buffer.line_count() {
            return &self.scratch;
        }
        let mut state = self.state_at(buffer, line);
        let text = buffer.line(line).unwrap_or("");
        tokenize_spans(text, self.lang, &mut state, &mut self.scratch);
        // Record the resulting state for the next line so sequential
        // painting (the common case) never re-scans.
        //
        // `state_at` above leaves `valid_len == line + 1`, so the entry
        // this pushes is the one for `line + 1`. Testing `line ==
        // valid_len` instead never matched for a block-comment language
        // — every painted line was tokenized twice — and for a language
        // without block comments it pushed entries `state_at` never
        // reads.
        if self.valid_len == line + 1 {
            self.states.push(state);
            self.valid_len += 1;
        }
        &self.scratch
    }

    /// Block-comment flag at the start of `line`, extending the cache as
    /// needed.
    fn state_at(&mut self, buffer: &EditorBuffer, line: usize) -> bool {
        if !self.lang.has_block_comments() {
            return false;
        }
        if line < self.valid_len {
            return self.states[line];
        }
        // states[n] is the state entering line n; the entry for line 0 is
        // always false.
        if self.states.is_empty() {
            self.states.push(false);
            self.valid_len = 1;
        }
        let mut scratch = std::mem::take(&mut self.scratch);
        while self.valid_len <= line {
            let idx = self.valid_len - 1;
            let mut state = self.states[idx];
            let text = buffer.line(idx).unwrap_or("");
            tokenize_spans(text, self.lang, &mut state, &mut scratch);
            self.states.push(state);
            self.valid_len += 1;
        }
        scratch.clear();
        self.scratch = scratch;
        self.states[line]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::TokenKind;

    fn buf(text: &str) -> EditorBuffer {
        EditorBuffer::from_text(text)
    }

    fn kinds(cache: &mut SyntaxCache, buffer: &EditorBuffer, line: usize) -> Vec<TokenKind> {
        cache
            .spans_for(buffer, line)
            .iter()
            .map(|s| s.kind)
            .collect()
    }

    /// `state_at` leaves `valid_len == line + 1`, so the record-forward
    /// branch has to test that, not `line == valid_len` — which never
    /// matched, and made every painted line tokenize twice.
    #[test]
    fn painting_lines_in_order_extends_the_cache_by_one_each_time() {
        let b = buf("/* a\n b\n c */\nlet x = 1;\nlet y = 2;");
        let mut cache = SyntaxCache::new(Language::Rust);
        for line in 0..b.line_count() {
            cache.spans_for(&b, line);
            assert_eq!(
                cache.valid_len,
                line + 2,
                "line {line} must leave the next line's state cached"
            );
        }
    }

    /// A language with no block comments has no state to carry, so the
    /// cache must not grow a `bool` per painted line that nothing reads.
    #[test]
    fn a_language_without_block_comments_caches_nothing() {
        let b = buf("echo one\necho two\necho three");
        let mut cache = SyntaxCache::new(Language::Shell);
        for line in 0..b.line_count() {
            cache.spans_for(&b, line);
        }
        assert_eq!(cache.valid_len, 0);
        assert!(cache.states.is_empty());
    }

    #[test]
    fn plain_text_produces_no_spans() {
        let b = buf("let x = 1;");
        let mut cache = SyntaxCache::new(Language::Plain);
        assert!(!cache.is_active());
        assert!(cache.spans_for(&b, 0).is_empty());
    }

    #[test]
    fn spans_cover_the_whole_line_without_gaps() {
        let b = buf("let x = \"hi\"; // done");
        let mut cache = SyntaxCache::new(Language::Rust);
        let spans = cache.spans_for(&b, 0);
        assert!(!spans.is_empty());
        let mut at = 0usize;
        for span in spans {
            assert_eq!(span.start, at, "gap or overlap before {span:?}");
            assert!(span.end > span.start, "empty span {span:?}");
            at = span.end;
        }
        assert_eq!(at, b.line(0).unwrap().len(), "spans must reach the end");
    }

    /// The whole reason this cache exists: painting a viewport that
    /// starts in the middle of a block comment must know it is inside
    /// one, which is only knowable by scanning from the top.
    #[test]
    fn a_line_inside_a_block_comment_is_all_comment() {
        let b = buf("/* opening\nstill comment\nalso comment\n*/ let x = 1;");
        let mut cache = SyntaxCache::new(Language::Rust);

        // Jump straight to line 2 without painting 0 and 1 first.
        let inside = kinds(&mut cache, &b, 2);
        assert!(
            inside.iter().all(|k| *k == TokenKind::Comment),
            "line inside the block must be entirely comment, got {inside:?}"
        );

        // The line that closes the comment goes back to code.
        let closing = kinds(&mut cache, &b, 3);
        assert!(
            closing.contains(&TokenKind::Keyword),
            "code after the close must highlight again, got {closing:?}"
        );
    }

    #[test]
    fn editing_above_invalidates_the_state_below() {
        let mut b = buf("let a = 1;\nlet b = 2;\nlet c = 3;");
        let mut cache = SyntaxCache::new(Language::Rust);
        assert!(kinds(&mut cache, &b, 2).contains(&TokenKind::Keyword));

        // Open a block comment on line 0; line 2 is now inside it.
        b = buf("/* let a = 1;\nlet b = 2;\nlet c = 3;");
        cache.invalidate_from(0);
        let after = kinds(&mut cache, &b, 2);
        assert!(
            after.iter().all(|k| *k == TokenKind::Comment),
            "state below the edit must be recomputed, got {after:?}"
        );
    }

    #[test]
    fn languages_without_block_comments_skip_the_walk() {
        let b = buf("# one\n# two\necho three");
        let mut cache = SyntaxCache::new(Language::Shell);
        // Asking for a far line must not require the walk; the state is
        // trivially false, so nothing is cached on the way.
        let spans = cache.spans_for(&b, 2);
        assert!(!spans.is_empty());
        assert!(
            cache.valid_len <= 1,
            "no line-by-line walk should have happened, valid_len={}",
            cache.valid_len
        );
    }

    #[test]
    fn out_of_range_lines_are_empty() {
        let b = buf("one line");
        let mut cache = SyntaxCache::new(Language::Rust);
        assert!(cache.spans_for(&b, 99).is_empty());
    }
}
