//! In-file search for the editor and diff panes.
//!
//! Plain substring search, no regex: the goal is "where else does this
//! identifier appear", and a regex engine is a dependency this crate does
//! not have. Matching follows the smart-case convention every modern
//! editor uses — a lowercase query matches case-insensitively, a query
//! with any uppercase letter matches exactly — with an explicit toggle to
//! force case sensitivity.
//!
//! The whole match set is computed up front (bounded by [`MAX_MATCHES`])
//! rather than lazily: the pane needs the total for the `3 of 12`
//! counter, painting needs the matches on each visible line, and a
//! document is at most [`crate::editor::MAX_EDITOR_FILE_BYTES`].

use super::buffer::{EditorBuffer, Position};

/// Upper bound on recorded matches. A query like `e` over a large file
/// would otherwise allocate millions of entries for a counter nobody
/// reads; past this the pane shows `10000+` and navigation still works
/// over the matches it has.
pub const MAX_MATCHES: usize = 10_000;

/// One match, as a byte range inside a single buffer line. Matches never
/// span lines (the query is a single line of text).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Match {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

impl Match {
    pub fn position(&self) -> Position {
        Position {
            line: self.line,
            col: self.start,
        }
    }
}

/// Find-bar state for one pane. `None` on the pane means the bar is
/// closed; an open bar with an empty query shows no matches and no
/// counter, which is what the user sees the instant they press Ctrl+F.
#[derive(Clone, Debug, Default)]
pub struct FindState {
    pub query: String,
    /// Explicit "Aa" toggle. When off, smart case applies.
    pub case_sensitive: bool,
    pub matches: Vec<Match>,
    /// Index into `matches` of the highlighted match. Meaningless when
    /// `matches` is empty.
    pub current: usize,
    /// The match list hit [`MAX_MATCHES`] and is not exhaustive.
    pub capped: bool,
}

impl FindState {
    /// Does the query match case-sensitively? Smart case: any uppercase
    /// letter in the query means the user typed a specific capitalisation
    /// and meant it.
    pub fn effective_case_sensitive(&self) -> bool {
        self.case_sensitive || self.query.chars().any(|c| c.is_uppercase())
    }

    /// Recompute `matches` for the current query, keeping the selected
    /// match as close as possible to `near` (normally the cursor) so
    /// editing the query does not throw the user back to the top of the
    /// file.
    pub fn refresh(&mut self, buffer: &EditorBuffer, near: Position) {
        let case_sensitive = self.effective_case_sensitive();
        let (matches, capped) = search(buffer, &self.query, case_sensitive);
        self.matches = matches;
        self.capped = capped;
        self.current = self.index_at_or_after(near);
    }

    /// First match at or after `pos`, wrapping to 0. Zero when there are
    /// no matches at all.
    pub fn index_at_or_after(&self, pos: Position) -> usize {
        if self.matches.is_empty() {
            return 0;
        }
        self.matches
            .iter()
            .position(|m| m.position() >= pos)
            .unwrap_or(0)
    }

    /// Advance to the next match, wrapping at the end. Returns the match
    /// to reveal, or `None` when there is nothing to find.
    pub fn next(&mut self) -> Option<Match> {
        if self.matches.is_empty() {
            return None;
        }
        self.current = (self.current + 1) % self.matches.len();
        self.matches.get(self.current).copied()
    }

    /// Step back to the previous match, wrapping at the start.
    pub fn prev(&mut self) -> Option<Match> {
        if self.matches.is_empty() {
            return None;
        }
        self.current = if self.current == 0 {
            self.matches.len() - 1
        } else {
            self.current - 1
        };
        self.matches.get(self.current).copied()
    }

    pub fn current_match(&self) -> Option<Match> {
        self.matches.get(self.current).copied()
    }

    /// Matches on `line`, as `(start, end)` byte ranges, for painting.
    /// The list is sorted, so this is a contiguous slice scan.
    pub fn matches_on_line(&self, line: usize) -> impl Iterator<Item = &Match> {
        self.matches.iter().filter(move |m| m.line == line)
    }

    /// `3 of 12`, `10000+ matches`, or `No results` — the counter shown
    /// in the bar. Empty string while the query is empty.
    pub fn counter_label(&self) -> String {
        if self.query.is_empty() {
            return String::new();
        }
        if self.matches.is_empty() {
            return "No results".to_string();
        }
        if self.capped {
            return format!("{} of {}+", self.current + 1, self.matches.len());
        }
        format!("{} of {}", self.current + 1, self.matches.len())
    }
}

/// All matches of `query` in `buffer`, in document order, plus whether
/// the [`MAX_MATCHES`] cap truncated the list.
pub fn search(buffer: &EditorBuffer, query: &str, case_sensitive: bool) -> (Vec<Match>, bool) {
    let mut out = Vec::new();
    if query.is_empty() {
        return (out, false);
    }
    // Lowercasing per line (rather than per candidate position) keeps the
    // scan linear; the needle is lowered once.
    let needle = if case_sensitive {
        query.to_string()
    } else {
        query.to_lowercase()
    };
    let mut haystack = String::new();
    for line_idx in 0..buffer.line_count() {
        let Some(line) = buffer.line(line_idx) else {
            continue;
        };
        let text: &str = if case_sensitive {
            line
        } else {
            haystack.clear();
            haystack.extend(line.chars().flat_map(|c| c.to_lowercase()));
            &haystack
        };
        // `to_lowercase` can change a character's byte length (İ -> i̇), so
        // offsets from a lowercased haystack are only safe to report when
        // the lengths agree. They almost always do; when they don't, fall
        // back to a case-sensitive scan of that line rather than reporting
        // a byte range that would panic on slicing.
        let offsets_valid = case_sensitive || text.len() == line.len();
        let (scan, needle_ref): (&str, &str) = if offsets_valid {
            (text, &needle)
        } else {
            (line, query)
        };
        let mut from = 0usize;
        while let Some(found) = scan[from..].find(needle_ref) {
            let start = from + found;
            let end = start + needle_ref.len();
            out.push(Match {
                line: line_idx,
                start,
                end,
            });
            if out.len() >= MAX_MATCHES {
                return (out, true);
            }
            // Overlapping matches are not reported: advance past this one,
            // but never by zero (an empty needle is rejected above).
            from = end;
            if from > scan.len() {
                break;
            }
        }
    }
    (out, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(text: &str) -> EditorBuffer {
        EditorBuffer::from_text(text)
    }

    #[test]
    fn finds_every_occurrence_in_document_order() {
        let b = buf("alpha beta\nbeta gamma\nno match here\nbeta");
        let (matches, capped) = search(&b, "beta", true);
        assert!(!capped);
        assert_eq!(
            matches,
            vec![
                Match {
                    line: 0,
                    start: 6,
                    end: 10
                },
                Match {
                    line: 1,
                    start: 0,
                    end: 4
                },
                Match {
                    line: 3,
                    start: 0,
                    end: 4
                },
            ]
        );
    }

    #[test]
    fn repeated_matches_on_one_line_do_not_overlap() {
        let b = buf("aaaa");
        let (matches, _) = search(&b, "aa", true);
        assert_eq!(matches.len(), 2, "non-overlapping scan: got {matches:?}");
        assert_eq!(matches[1].start, 2);
    }

    /// Smart case is the whole reason the toggle exists: typing a
    /// lowercase word must find the capitalised one, but typing a
    /// capital must not drag in the lowercase noise.
    #[test]
    fn smart_case_matches_loosely_until_the_query_has_a_capital() {
        let b = buf("Editor editor EDITOR");

        let mut state = FindState {
            query: "editor".to_string(),
            ..Default::default()
        };
        assert!(!state.effective_case_sensitive());
        state.refresh(&b, Position { line: 0, col: 0 });
        assert_eq!(state.matches.len(), 3);

        state.query = "Editor".to_string();
        assert!(state.effective_case_sensitive());
        state.refresh(&b, Position { line: 0, col: 0 });
        assert_eq!(state.matches.len(), 1);
        assert_eq!(state.matches[0].start, 0);
    }

    #[test]
    fn explicit_toggle_forces_case_sensitivity_for_a_lowercase_query() {
        let b = buf("Editor editor");
        let mut state = FindState {
            query: "editor".to_string(),
            case_sensitive: true,
            ..Default::default()
        };
        state.refresh(&b, Position { line: 0, col: 0 });
        assert_eq!(state.matches.len(), 1);
        assert_eq!(state.matches[0].start, 7);
    }

    #[test]
    fn case_insensitive_offsets_point_at_the_original_bytes() {
        // The lowercased haystack must not shift the reported range, or
        // the painter would tint the wrong cells on a non-ASCII line.
        let b = buf("CAFÉ café");
        let mut state = FindState {
            query: "café".to_string(),
            ..Default::default()
        };
        state.refresh(&b, Position { line: 0, col: 0 });
        assert_eq!(state.matches.len(), 2);
        let line = b.line(0).unwrap();
        for m in &state.matches {
            assert!(
                line.is_char_boundary(m.start) && line.is_char_boundary(m.end),
                "range {m:?} must land on char boundaries of {line:?}"
            );
            assert_eq!(line[m.start..m.end].to_lowercase(), "café");
        }
    }

    #[test]
    fn next_and_prev_wrap_around() {
        let b = buf("x\nx\nx");
        let mut state = FindState {
            query: "x".to_string(),
            ..Default::default()
        };
        state.refresh(&b, Position { line: 0, col: 0 });
        assert_eq!(state.current, 0);
        assert_eq!(state.next().map(|m| m.line), Some(1));
        assert_eq!(state.next().map(|m| m.line), Some(2));
        assert_eq!(state.next().map(|m| m.line), Some(0), "wraps to the top");
        assert_eq!(state.prev().map(|m| m.line), Some(2), "wraps backwards");
    }

    /// Opening the bar mid-file must select the match the user is looking
    /// at, not send them back to line 1.
    #[test]
    fn refresh_selects_the_match_at_or_after_the_cursor() {
        let b = buf("hit\nfiller\nhit\nfiller\nhit");
        let mut state = FindState {
            query: "hit".to_string(),
            ..Default::default()
        };
        state.refresh(&b, Position { line: 2, col: 0 });
        assert_eq!(state.current, 1);
        assert_eq!(state.current_match().unwrap().line, 2);

        // Past the last match it wraps to the first rather than dangling.
        state.refresh(&b, Position { line: 99, col: 0 });
        assert_eq!(state.current, 0);
    }

    #[test]
    fn empty_query_matches_nothing_and_shows_no_counter() {
        let b = buf("anything");
        let mut state = FindState::default();
        state.refresh(&b, Position { line: 0, col: 0 });
        assert!(state.matches.is_empty());
        assert!(state.next().is_none());
        assert_eq!(state.counter_label(), "");
    }

    #[test]
    fn counter_label_reports_position_total_and_absence() {
        let b = buf("a a a");
        let mut state = FindState {
            query: "a".to_string(),
            ..Default::default()
        };
        state.refresh(&b, Position { line: 0, col: 0 });
        assert_eq!(state.counter_label(), "1 of 3");
        state.next();
        assert_eq!(state.counter_label(), "2 of 3");

        state.query = "zzz".to_string();
        state.refresh(&b, Position { line: 0, col: 0 });
        assert_eq!(state.counter_label(), "No results");
    }

    /// A one-character query over a big document must not allocate
    /// without bound.
    #[test]
    fn match_list_is_capped() {
        let line = "x".repeat(200);
        let text = std::iter::repeat_n(line.as_str(), 200)
            .collect::<Vec<_>>()
            .join("\n");
        let b = buf(&text);
        let (matches, capped) = search(&b, "x", true);
        assert!(capped, "40 000 matches must trip the cap");
        assert_eq!(matches.len(), MAX_MATCHES);

        let mut state = FindState {
            query: "x".to_string(),
            ..Default::default()
        };
        state.refresh(&b, Position { line: 0, col: 0 });
        assert!(state.counter_label().ends_with('+'), "cap is surfaced");
    }

    #[test]
    fn matches_on_line_filters_to_that_line() {
        let b = buf("a b a\nc\na");
        let (matches, _) = search(&b, "a", true);
        let state = FindState {
            query: "a".to_string(),
            matches,
            ..Default::default()
        };
        assert_eq!(state.matches_on_line(0).count(), 2);
        assert_eq!(state.matches_on_line(1).count(), 0);
        assert_eq!(state.matches_on_line(2).count(), 1);
    }
}
