//! The diff-pane half of an [`EditorPane`](super::EditorPane).
//!
//! A diff pane is an editor pane with a read-only buffer of stacked
//! unified diffs and one decoration per row. Everything else — viewport,
//! wheel scrolling, mouse selection, copy, find, go-to-line, resize,
//! close, persistence stripping — is inherited rather than reimplemented,
//! which is why the diff viewer is a *mode* of the editor and not a
//! fourth kind of pane.

use std::path::PathBuf;

use unshit::core::style::types::Color;

use crate::diff::{DiffDocument, DiffFileInfo, DiffRowInfo, DiffRowKind, DiffSpec};
use crate::syntax::Language;

use super::colors::EditorColors;

/// What an editor pane is showing.
pub enum EditorKind {
    /// A file on disk, editable and saveable.
    File,
    /// A git range, read-only. Boxed because `EditorPane` is stored by
    /// value in a map and the diff payload is much larger than the file
    /// case.
    Diff(Box<DiffView>),
}

impl EditorKind {
    pub fn as_diff(&self) -> Option<&DiffView> {
        match self {
            EditorKind::Diff(view) => Some(view),
            EditorKind::File => None,
        }
    }

    pub fn as_diff_mut(&mut self) -> Option<&mut DiffView> {
        match self {
            EditorKind::Diff(view) => Some(view),
            EditorKind::File => None,
        }
    }
}

/// Where a diff pane's content is in its lifecycle. Git runs on a worker
/// thread, so the pane exists (and is scrollable, closeable and
/// focusable) before its content does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffLoad {
    Loading,
    Ready,
    /// git failed, or the range did not resolve. The message is already
    /// user-facing.
    Failed(String),
}

/// Block-comment state entering every row of a parsed document.
///
/// Only rows inside a hunk can inherit anything: a file header, a hunk
/// header, a spacer or a meta row resets both sides to "not in a
/// comment". The two sides are tracked apart because a hunk interleaves
/// them — a `/*` deleted on a `-` row must not comment out the `+` rows
/// that replace it — and they are advanced together on a context row,
/// which belongs to both. The common case (the sides agree) tokenizes
/// each row exactly once.
///
/// Files whose language has no block comments contribute `false` without
/// being tokenized at all, which is most of a typical diff.
fn block_states(
    lines: &[String],
    rows: &[crate::diff::DiffRowInfo],
    files: &[crate::diff::DiffFileInfo],
) -> Vec<bool> {
    let mut states = Vec::with_capacity(rows.len());
    let mut old_side = false;
    let mut new_side = false;
    let mut scratch = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let lang = files
            .get(row.file as usize)
            .map(|f| f.lang)
            .unwrap_or(Language::Plain);
        if !row.kind.is_content() || !lang.has_block_comments() {
            // Headers and spacers separate hunks; a language without
            // block comments can never be in one.
            old_side = false;
            new_side = false;
            states.push(false);
            continue;
        }
        let text = lines.get(index).map(String::as_str).unwrap_or("");
        let mut run = |state: &mut bool| {
            crate::syntax::tokenize_spans(text, lang, state, &mut scratch);
        };
        match row.kind {
            crate::diff::DiffRowKind::Added => {
                states.push(new_side);
                run(&mut new_side);
            }
            crate::diff::DiffRowKind::Removed => {
                states.push(old_side);
                run(&mut old_side);
            }
            // Context: the same text on both sides. Colour it by the new
            // side, which is the version the reviewer is reading.
            _ => {
                states.push(new_side);
                if old_side == new_side {
                    run(&mut new_side);
                    old_side = new_side;
                } else {
                    run(&mut new_side);
                    run(&mut old_side);
                }
            }
        }
    }
    states
}

/// A diff pane's decorations and navigation index.
pub struct DiffView {
    pub spec: DiffSpec,
    pub repo_root: PathBuf,
    /// Correlates this pane's telemetry with the job that filled it.
    pub job_id: String,
    pub load: DiffLoad,
    /// Parallel to the buffer's lines. Empty while loading.
    pub rows: Vec<DiffRowInfo>,
    pub files: Vec<DiffFileInfo>,
    /// Cached from the document so the painter does not rescan.
    gutter_w: usize,
    old_digits: usize,
    new_digits: usize,
    /// Block-comment state entering each row, parallel to `rows`.
    ///
    /// A diff is not a contiguous document but a hunk is, so this is
    /// computed once at load: reset at every header, and tracked per side
    /// so a `/*` on a removed line does not colour the added lines that
    /// replace it.
    block_states: Vec<bool>,
    pub truncated: bool,
}

impl DiffView {
    /// A view with no content yet, shown while git runs.
    pub fn loading(spec: DiffSpec, repo_root: PathBuf, job_id: String) -> Self {
        Self {
            spec,
            repo_root,
            job_id,
            load: DiffLoad::Loading,
            rows: Vec::new(),
            files: Vec::new(),
            // The placeholder buffer has no numbers to show; a narrow
            // blank gutter keeps the text where it will be once loaded.
            gutter_w: 2,
            old_digits: 0,
            new_digits: 0,
            block_states: Vec::new(),
            truncated: false,
        }
    }

    /// Adopt a parsed document. The caller replaces the pane's buffer
    /// with `document.lines` in the same step — the two must stay in
    /// lockstep or the decorations would describe the wrong rows.
    pub fn adopt(&mut self, document: &DiffDocument) {
        let max_old = document
            .rows
            .iter()
            .filter_map(|r| r.old_no)
            .max()
            .unwrap_or(0);
        let max_new = document
            .rows
            .iter()
            .filter_map(|r| r.new_no)
            .max()
            .unwrap_or(0);
        let digits = |n: u32| (n.max(1).ilog10() as usize + 1).max(3);
        self.old_digits = digits(max_old);
        self.new_digits = digits(max_new);
        self.gutter_w = super::grid::diff_gutter_width(max_old, max_new);
        self.rows = document.rows.clone();
        self.files = document.files.clone();
        self.block_states = block_states(&document.lines, &document.rows, &document.files);
        self.truncated = document.truncated;
        self.load = DiffLoad::Ready;
    }

    /// Block-comment state entering `index`; `false` for any row outside
    /// a hunk, and for every row of a language without block comments.
    pub fn block_state_at(&self, index: usize) -> bool {
        self.block_states.get(index).copied().unwrap_or(false)
    }

    pub fn fail(&mut self, message: String) {
        self.load = DiffLoad::Failed(message);
        self.rows.clear();
        self.files.clear();
        self.block_states.clear();
        self.gutter_w = 2;
    }

    pub fn gutter_width(&self) -> usize {
        self.gutter_w
    }

    pub fn row(&self, index: usize) -> Option<&DiffRowInfo> {
        self.rows.get(index)
    }

    pub fn file_of_row(&self, index: usize) -> Option<&DiffFileInfo> {
        let row = self.rows.get(index)?;
        self.files.get(row.file as usize)
    }

    /// Language for syntax colouring on `index`, or `Plain` for the rows
    /// that are not source text.
    pub fn language_at(&self, index: usize) -> Language {
        match self.rows.get(index) {
            Some(row) if row.kind.is_content() => self
                .files
                .get(row.file as usize)
                .map(|f| f.lang)
                .unwrap_or(Language::Plain),
            _ => Language::Plain,
        }
    }

    /// Write the gutter for `index`: `old  new marker`, with a blank
    /// number column on the side the row does not exist in.
    pub fn gutter_text(&self, index: usize, out: &mut String) {
        out.clear();
        let Some(row) = self.rows.get(index) else {
            for _ in 0..self.gutter_w {
                out.push(' ');
            }
            return;
        };
        if matches!(
            row.kind,
            DiffRowKind::FileHeader | DiffRowKind::Spacer | DiffRowKind::Meta
        ) {
            // Headers read as headings: no numbers, no marker.
            for _ in 0..self.gutter_w {
                out.push(' ');
            }
            return;
        }
        push_number(out, row.old_no, self.old_digits);
        out.push(' ');
        push_number(out, row.new_no, self.new_digits);
        out.push(' ');
        out.push(row.kind.marker());
        out.push(' ');
    }

    /// Row background tint for `index`.
    pub fn row_bg(&self, index: usize, colors: &EditorColors) -> Option<Color> {
        match self.rows.get(index)?.kind {
            DiffRowKind::Added => Some(colors.diff_added_bg),
            DiffRowKind::Removed => Some(colors.diff_removed_bg),
            DiffRowKind::HunkHeader => Some(colors.diff_hunk_bg),
            DiffRowKind::FileHeader => Some(colors.diff_file_bg),
            DiffRowKind::Context | DiffRowKind::Meta | DiffRowKind::Spacer => None,
        }
    }

    /// Foreground override for `index`: header rows get one colour,
    /// content rows keep their syntax colours.
    pub fn fg_override(&self, index: usize, colors: &EditorColors) -> Option<Color> {
        match self.rows.get(index)?.kind {
            DiffRowKind::FileHeader => Some(colors.diff_file_fg),
            DiffRowKind::HunkHeader => Some(colors.diff_hunk_fg),
            DiffRowKind::Meta => Some(colors.diff_meta_fg),
            DiffRowKind::Context | DiffRowKind::Added | DiffRowKind::Removed => None,
            DiffRowKind::Spacer => None,
        }
    }

    pub fn is_bold(&self, index: usize) -> bool {
        self.rows
            .get(index)
            .is_some_and(|r| r.kind == DiffRowKind::FileHeader)
    }

    /// Colour of the gutter for `index`: the marker column carries the
    /// add/remove colour so the eye can scan the left edge.
    pub fn gutter_fg(&self, index: usize, colors: &EditorColors) -> Color {
        match self.rows.get(index).map(|r| r.kind) {
            Some(DiffRowKind::Added) => colors.diff_added_fg,
            Some(DiffRowKind::Removed) => colors.diff_removed_fg,
            _ => colors.gutter,
        }
    }

    /// Row index of the next hunk header strictly after `from`, in
    /// document order across files.
    pub fn next_hunk(&self, from: usize) -> Option<usize> {
        self.hunk_rows().find(|row| *row > from)
    }

    /// Row index of the previous hunk header strictly before `from`.
    pub fn prev_hunk(&self, from: usize) -> Option<usize> {
        self.hunk_rows().rfind(|row| *row < from)
    }

    /// Every hunk header row, ascending. Files are already in document
    /// order and each file's `hunk_rows` is ascending, so a flat map is
    /// sorted by construction.
    fn hunk_rows(&self) -> impl DoubleEndedIterator<Item = usize> + '_ {
        self.files.iter().flat_map(|f| f.hunk_rows.iter().copied())
    }

    pub fn next_file(&self, from: usize) -> Option<usize> {
        self.files
            .iter()
            .map(|f| f.first_row)
            .find(|row| *row > from)
    }

    pub fn prev_file(&self, from: usize) -> Option<usize> {
        self.files
            .iter()
            .map(|f| f.first_row)
            .rfind(|row| *row < from)
    }

    /// The file and new-file line a row points at, for "open this in an
    /// editor". A removed line has no new-file line of its own, so the
    /// nearest following new line in the same hunk is used — that is
    /// where the change landed.
    pub fn open_target(&self, index: usize) -> Option<(&DiffFileInfo, u32)> {
        let row = self.rows.get(index)?;
        let file = self.files.get(row.file as usize)?;
        if let Some(line) = row.new_no {
            return Some((file, line));
        }
        // Walk forward within the same file for a row that exists in the
        // new file; fall back to the last known new line before it.
        let forward = self.rows[index..]
            .iter()
            .take_while(|r| r.file == row.file)
            .find_map(|r| r.new_no);
        if let Some(line) = forward {
            return Some((file, line));
        }
        let backward = self.rows[..index]
            .iter()
            .rev()
            .take_while(|r| r.file == row.file)
            .find_map(|r| r.new_no);
        Some((file, backward.unwrap_or(1)))
    }

    /// First row of the file whose path ends with `rel`, for a Flow
    /// hand-off that names a file. Matching is suffix-based because the
    /// Flow document's paths are relative to *its* repo root, which is
    /// normally the same tree but may be spelled differently.
    pub fn find_file_row(&self, rel: &str) -> Option<usize> {
        let needle = rel.replace('\\', "/");
        let needle = needle.trim_start_matches("./");
        self.files
            .iter()
            .find(|f| f.path == needle || f.path.ends_with(&format!("/{needle}")))
            .map(|f| f.first_row)
    }

    /// The row showing `line` of `rel`'s new file, or the file's header
    /// when the line is not part of the diff.
    pub fn find_line_row(&self, rel: &str, line: u32) -> Option<usize> {
        let first = self.find_file_row(rel)?;
        let file_idx = self.rows.get(first)?.file;
        let exact = self
            .rows
            .iter()
            .enumerate()
            .skip(first)
            .take_while(|(_, r)| r.file == file_idx)
            .find(|(_, r)| r.new_no == Some(line))
            .map(|(idx, _)| idx);
        Some(exact.unwrap_or(first))
    }
}

/// Right-align `value` in `width` cells, blank when the row does not
/// exist on that side.
fn push_number(out: &mut String, value: Option<u32>, width: usize) {
    match value {
        Some(n) => {
            let text = n.to_string();
            for _ in text.len()..width {
                out.push(' ');
            }
            out.push_str(&text);
        }
        None => {
            for _ in 0..width {
                out.push(' ');
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::parse_unified_diff;

    /// A two-file diff. Built by joining lines rather than written as one
    /// literal: a unified diff's context lines *start with a space*, and
    /// Rust's `\` line continuation would eat exactly that.
    fn sample() -> String {
        [
            "diff --git a/src/a.rs b/src/a.rs",
            "index 111..222 100644",
            "--- a/src/a.rs",
            "+++ b/src/a.rs",
            "@@ -1,3 +1,4 @@",
            " fn main() {",
            "-    let x = 1;",
            "+    let x = 2;",
            "+    let y = 3;",
            " }",
            "diff --git a/src/b.rs b/src/b.rs",
            "index 333..444 100644",
            "--- a/src/b.rs",
            "+++ b/src/b.rs",
            "@@ -10,2 +10,2 @@",
            "-old",
            "+new",
            "",
        ]
        .join("\n")
    }

    fn view() -> (DiffView, DiffDocument) {
        let doc = parse_unified_diff(&sample());
        let mut view = DiffView::loading(
            DiffSpec::parse("HEAD").expect("spec"),
            PathBuf::from("C:/repo"),
            "diff-test".to_string(),
        );
        view.adopt(&doc);
        (view, doc)
    }

    #[test]
    fn adopting_a_document_takes_its_rows_and_files() {
        let (view, doc) = view();
        assert_eq!(view.load, DiffLoad::Ready);
        assert_eq!(view.rows.len(), doc.lines.len());
        assert_eq!(view.files.len(), 2);
    }

    /// The gutter is the diff's whole navigation aid: two number columns
    /// and a marker, with a blank on the side a row does not exist in.
    #[test]
    fn gutter_shows_old_and_new_numbers_with_a_marker() {
        let (view, _) = view();
        let mut out = String::new();
        let added = view
            .rows
            .iter()
            .position(|r| r.kind == DiffRowKind::Added)
            .expect("an added row");
        view.gutter_text(added, &mut out);
        assert_eq!(out.len(), view.gutter_width(), "gutter is fixed width");
        assert!(out.ends_with("+ "), "added rows carry a + marker: {out:?}");
        assert!(
            out.trim_start().starts_with(char::is_numeric) || out.starts_with(' '),
            "old column is blank for an added row: {out:?}"
        );

        let removed = view
            .rows
            .iter()
            .position(|r| r.kind == DiffRowKind::Removed)
            .expect("a removed row");
        view.gutter_text(removed, &mut out);
        assert!(out.ends_with("- "), "{out:?}");

        let header = view
            .rows
            .iter()
            .position(|r| r.kind == DiffRowKind::FileHeader)
            .expect("a file header");
        view.gutter_text(header, &mut out);
        assert!(
            out.trim().is_empty(),
            "headers have a blank gutter: {out:?}"
        );
    }

    #[test]
    fn tints_and_overrides_follow_the_row_kind() {
        let (view, _) = view();
        let colors = EditorColors::default();
        let find = |kind: DiffRowKind| view.rows.iter().position(|r| r.kind == kind).unwrap();

        assert_eq!(
            view.row_bg(find(DiffRowKind::Added), &colors),
            Some(colors.diff_added_bg)
        );
        assert_eq!(
            view.row_bg(find(DiffRowKind::Removed), &colors),
            Some(colors.diff_removed_bg)
        );
        assert_eq!(view.row_bg(find(DiffRowKind::Context), &colors), None);
        assert_eq!(
            view.fg_override(find(DiffRowKind::HunkHeader), &colors),
            Some(colors.diff_hunk_fg)
        );
        assert_eq!(
            view.fg_override(find(DiffRowKind::Added), &colors),
            None,
            "content rows keep syntax colours"
        );
        assert!(view.is_bold(find(DiffRowKind::FileHeader)));
    }

    #[test]
    fn only_content_rows_get_a_language() {
        let (view, _) = view();
        let header = view
            .rows
            .iter()
            .position(|r| r.kind == DiffRowKind::FileHeader)
            .unwrap();
        let content = view
            .rows
            .iter()
            .position(|r| r.kind == DiffRowKind::Added)
            .unwrap();
        assert_eq!(view.language_at(header), Language::Plain);
        assert_eq!(view.language_at(content), Language::Rust);
    }

    /// Hunk navigation must cross file boundaries in document order:
    /// pressing `n` at the end of one file lands in the next.
    #[test]
    fn hunk_navigation_walks_every_hunk_in_order() {
        let (view, _) = view();
        let hunks: Vec<usize> = view.hunk_rows().collect();
        assert_eq!(hunks.len(), 2, "one hunk per file in the fixture");
        assert!(hunks[0] < hunks[1]);

        assert_eq!(view.next_hunk(0), Some(hunks[0]));
        assert_eq!(view.next_hunk(hunks[0]), Some(hunks[1]));
        assert_eq!(view.next_hunk(hunks[1]), None, "stops at the last hunk");
        assert_eq!(view.prev_hunk(hunks[1]), Some(hunks[0]));
        assert_eq!(view.prev_hunk(hunks[0]), None);
    }

    #[test]
    fn file_navigation_jumps_between_headers() {
        let (view, _) = view();
        let first = view.files[0].first_row;
        let second = view.files[1].first_row;
        assert_eq!(view.next_file(first), Some(second));
        assert_eq!(view.next_file(second), None);
        assert_eq!(view.prev_file(second), Some(first));
    }

    /// Opening a removed line has to land somewhere sensible in the new
    /// file — the line is gone, so the change's location is the nearest
    /// following new line.
    #[test]
    fn open_target_resolves_a_new_file_line_for_every_row_kind() {
        let (view, _) = view();
        for (idx, row) in view.rows.iter().enumerate() {
            if matches!(row.kind, DiffRowKind::Spacer) {
                continue;
            }
            let (file, line) = view.open_target(idx).expect("every row resolves a target");
            assert!(line >= 1, "1-based line for row {idx} ({:?})", row.kind);
            assert!(!file.path.is_empty());
        }
        let removed = view
            .rows
            .iter()
            .position(|r| r.kind == DiffRowKind::Removed)
            .unwrap();
        let (_, line) = view.open_target(removed).unwrap();
        assert_eq!(line, 2, "the replacement line in the new file");
    }

    #[test]
    fn a_flow_handoff_finds_its_file_and_line() {
        let (view, _) = view();
        let row = view.find_file_row("src/a.rs").expect("file located");
        assert_eq!(row, view.files[0].first_row);
        // Suffix matching: a Flow document's paths are relative to its own
        // repo root, which may be spelled differently from the diff's.
        assert_eq!(view.find_file_row("a.rs"), Some(row));
        assert!(
            view.find_file_row("other/a.rs").is_none(),
            "a suffix must align on a path separator"
        );
        assert!(view.find_file_row("src/missing.rs").is_none());

        let line_row = view.find_line_row("src/a.rs", 2).expect("line located");
        assert_eq!(view.rows[line_row].new_no, Some(2));
        // A line outside the diff falls back to the file header.
        assert_eq!(
            view.find_line_row("src/a.rs", 9000),
            Some(view.files[0].first_row)
        );
    }

    #[test]
    fn a_failed_view_keeps_no_stale_rows() {
        let (mut view, _) = view();
        view.fail("bad revision".to_string());
        assert_eq!(view.load, DiffLoad::Failed("bad revision".to_string()));
        assert!(view.rows.is_empty());
        assert!(view.files.is_empty());
    }
}
