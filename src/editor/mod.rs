//! Built-in file editor: pane model, file open/save, viewport state.
//!
//! Editor panes live beside terminal panes in the tab/split layout. The
//! pane owns a pure `EditorBuffer` plus a live `CellGrid` painted with
//! the visible window only, mirroring how terminals publish their
//! display grid — the render tree never sees the whole file.

pub mod buffer;
pub mod colors;
pub mod diff_view;
pub mod find;
pub mod grid;
pub mod highlight;
pub mod telemetry;

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use unshit::core::cell_grid::CellGrid;

pub use buffer::{Damage, EditorBuffer, LineEnding, Position, TAB_SPACES};
pub use colors::EditorColors;
pub use diff_view::{DiffLoad, DiffView, EditorKind};
pub use find::FindState;
use grid::RowPaint;
use highlight::SyntaxCache;

/// Files above this size are refused (MVP guard; see SPEC.md).
pub const MAX_EDITOR_FILE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug)]
pub enum OpenError {
    TooLarge(u64),
    InvalidUtf8,
    Io(std::io::Error),
}

impl OpenError {
    /// Machine-readable reason for telemetry.
    pub fn reason(&self) -> &'static str {
        match self {
            OpenError::TooLarge(_) => "too_large",
            OpenError::InvalidUtf8 => "invalid_utf8",
            OpenError::Io(_) => "io",
        }
    }

    /// Human-readable message for the failure toast.
    pub fn message(&self, path: &Path) -> String {
        let name = display_name(path);
        match self {
            OpenError::TooLarge(bytes) => format!(
                "{} is too large to edit ({} MiB, limit {} MiB)",
                name,
                bytes / (1024 * 1024),
                MAX_EDITOR_FILE_BYTES / (1024 * 1024)
            ),
            OpenError::InvalidUtf8 => format!("{} is not valid UTF-8 text", name),
            OpenError::Io(e) => format!("Could not open {}: {}", name, e),
        }
    }
}

/// File name for titles; falls back to the full path string.
pub fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

pub(crate) fn generate_correlation_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!(
        "ed-{}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed),
        telemetry::now_unix_ms()
    )
}

/// Read and normalize a file for editing. Refuses oversized files and
/// invalid UTF-8 rather than silently converting (see SPEC boundaries).
pub fn load_file(path: &Path) -> Result<(EditorBuffer, LineEnding, u64), OpenError> {
    let metadata = std::fs::metadata(path).map_err(OpenError::Io)?;
    let size = metadata.len();
    if size > MAX_EDITOR_FILE_BYTES {
        return Err(OpenError::TooLarge(size));
    }
    let bytes = std::fs::read(path).map_err(OpenError::Io)?;
    let text = String::from_utf8(bytes).map_err(|_| OpenError::InvalidUtf8)?;
    let line_ending = LineEnding::detect(&text);
    // Normalize CRLF pairs first, then any stray bare CR (classic-Mac
    // or mixed files) so no line ever carries a literal `\r` — the
    // buffer invariant every consumer relies on. Bare CRs become line
    // breaks, matching VS Code and the typed/paste insert path.
    let normalized = if text.contains('\r') {
        text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        text
    };
    Ok((EditorBuffer::from_text(&normalized), line_ending, size))
}

/// A single editor pane: buffer + viewport + live paint grid.
pub struct EditorPane {
    pub path: PathBuf,
    pub display_name: String,
    pub buffer: EditorBuffer,
    /// First buffer line visible at the top of the viewport.
    pub top_line: usize,
    /// Visual columns (tabs expanded) skipped at the left of every line
    /// (horizontal scroll).
    pub h_offset: usize,
    pub dirty: bool,
    /// Undo-stack identity at the last save (0 = pristine). Comparing
    /// against `buffer.top_group_id()` yields the dirty flag correctly
    /// across undo/redo (undoing back to the saved state is clean).
    saved_top_group: u64,
    /// Correlates open→save→close telemetry for this pane instance.
    pub correlation_id: String,
    pub line_ending: LineEnding,
    /// On-disk size at open time (telemetry only; never content).
    pub file_bytes: u64,
    /// Live viewport grid, cloned into the render tree each frame just
    /// like a terminal's display grid.
    pub grid: CellGrid,
    /// Resolved from the active theme when the pane opens and again on
    /// every theme change; editor grids bypass the terminal palette
    /// remap, so colours have to be real at paint time.
    pub colors: EditorColors,
    /// Refuses every buffer mutation and every save. Diff panes are
    /// read-only; a file pane never is.
    pub read_only: bool,
    pub kind: EditorKind,
    /// `Some` while the find bar is open.
    pub find: Option<FindState>,
    /// Block-comment state per line, extended lazily as the viewport
    /// moves and invalidated from the first line each edit damages.
    syntax: SyntaxCache,
    /// Parsed Markdown presentation, dropped whenever the source changes so
    /// ordinary snapshots reuse it.
    markdown_cache: OnceLock<Arc<crate::markdown::MarkdownDocument>>,
    /// Whether a Markdown editor shows its rendered side pane.
    markdown_preview_open: bool,
}

impl EditorPane {
    pub fn open(path: &Path, rows: usize, cols: usize) -> Result<Self, OpenError> {
        Self::open_with_colors(path, rows, cols, EditorColors::default())
    }

    /// Open `path` with an explicit palette. The app always uses this so
    /// a new pane matches the current theme from its first frame.
    pub fn open_with_colors(
        path: &Path,
        rows: usize,
        cols: usize,
        colors: EditorColors,
    ) -> Result<Self, OpenError> {
        let (buffer, line_ending, file_bytes) = load_file(path)?;
        let language = crate::syntax::Language::from_path(&path.to_string_lossy());
        let mut pane = Self {
            path: path.to_path_buf(),
            display_name: display_name(path),
            buffer,
            top_line: 0,
            h_offset: 0,
            dirty: false,
            saved_top_group: 0,
            correlation_id: generate_correlation_id(),
            line_ending,
            file_bytes,
            grid: CellGrid::new(rows.max(1), cols.max(1)),
            colors,
            read_only: false,
            kind: EditorKind::File,
            find: None,
            syntax: SyntaxCache::new(language),
            markdown_cache: OnceLock::new(),
            markdown_preview_open: language == crate::syntax::Language::Markdown,
        };
        pane.repaint_viewport();
        pane.sync_cursor_into_grid();
        Ok(pane)
    }

    /// A diff pane with nothing in it yet. The tab appears immediately
    /// and git fills it in from a worker thread, so a slow diff never
    /// blocks the UI.
    pub fn diff_loading(
        spec: crate::diff::DiffSpec,
        repo_root: PathBuf,
        job_id: String,
        rows: usize,
        cols: usize,
        colors: EditorColors,
    ) -> Self {
        let label = spec.label();
        let mut pane = Self {
            // Diff panes have no file of their own. The repo root stands
            // in so `path`-keyed logic (duplicate-open checks, telemetry)
            // has something stable and non-empty.
            path: repo_root.clone(),
            display_name: format!("diff: {label}"),
            buffer: EditorBuffer::from_text(&format!("Loading diff {label}…")),
            top_line: 0,
            h_offset: 0,
            dirty: false,
            saved_top_group: 0,
            correlation_id: job_id.clone(),
            line_ending: LineEnding::Lf,
            file_bytes: 0,
            grid: CellGrid::new(rows.max(1), cols.max(1)),
            colors,
            read_only: true,
            kind: EditorKind::Diff(Box::new(DiffView::loading(spec, repo_root, job_id))),
            find: None,
            syntax: SyntaxCache::new(crate::syntax::Language::Plain),
            markdown_cache: OnceLock::new(),
            markdown_preview_open: false,
        };
        pane.repaint_viewport();
        pane.sync_cursor_into_grid();
        pane
    }

    /// Replace a diff pane's placeholder with a parsed document.
    /// Decorations and buffer lines are swapped together — they index
    /// each other, so they can never be set separately.
    pub fn set_diff_document(&mut self, document: &crate::diff::DiffDocument) {
        let Some(view) = self.kind.as_diff_mut() else {
            return;
        };
        view.adopt(document);
        let text = if document.lines.is_empty() {
            let label = view.spec.label();
            if view.spec.is_working_tree() {
                format!("No uncommitted changes against {label}.")
            } else {
                format!("No changes for {label}.")
            }
        } else {
            document.lines.join("\n")
        };
        self.replace_read_only_text(&text);
    }

    /// Show a failure in place of a diff pane's content.
    pub fn set_diff_error(&mut self, message: String) {
        let text = match self.kind.as_diff_mut() {
            Some(view) => {
                view.fail(message.clone());
                format!("Could not diff {}:\n{}", view.spec.label(), message)
            }
            None => message,
        };
        self.replace_read_only_text(&text);
    }

    /// Swap the whole buffer of a read-only pane and repaint. Undo
    /// history is reset with it: a diff document is not editable, so
    /// there is nothing to undo back to.
    fn replace_read_only_text(&mut self, text: &str) {
        self.buffer = EditorBuffer::from_text(text);
        self.saved_top_group = self.buffer.top_group_id();
        self.dirty = false;
        self.top_line = 0;
        self.h_offset = 0;
        self.syntax = SyntaxCache::new(crate::syntax::Language::Plain);
        if let Some(find) = self.find.as_mut() {
            find.refresh(&self.buffer, Position { line: 0, col: 0 });
        }
        self.repaint_viewport();
        self.sync_cursor_into_grid();
    }

    /// Re-resolve colours after a theme change and repaint everything.
    pub fn set_colors(&mut self, colors: EditorColors) {
        if self.colors == colors {
            return;
        }
        self.colors = colors;
        self.repaint_viewport();
        self.sync_cursor_into_grid();
    }

    /// The language highlighting was resolved to, which is also what
    /// decides the line-comment token for `editor.toggle_comment`.
    pub fn language(&self) -> crate::syntax::Language {
        self.syntax.language()
    }

    pub fn is_markdown(&self) -> bool {
        self.language() == crate::syntax::Language::Markdown
    }

    /// Return the current Markdown presentation, parsing at most once per
    /// content change. Cursor and selection-only changes reuse the cache.
    pub fn markdown_document(&self) -> Option<Arc<crate::markdown::MarkdownDocument>> {
        self.is_markdown().then(|| {
            Arc::clone(
                self.markdown_cache
                    .get_or_init(|| crate::markdown::parse(&self.buffer.to_text())),
            )
        })
    }

    /// How this pane presents Markdown, or `None` when it is not a
    /// Markdown editor.
    pub fn markdown_view(&self) -> Option<crate::markdown::MarkdownView> {
        use crate::markdown::MarkdownView;
        // Only parse while the preview is showing.
        let document = self
            .markdown_preview_open
            .then(|| self.markdown_document())
            .flatten();
        self.is_markdown()
            .then(|| document.map_or(MarkdownView::Closed, MarkdownView::Open))
    }

    /// Flip the side preview. Returns false when this is not a Markdown editor.
    pub fn toggle_markdown_preview(&mut self) -> bool {
        let markdown = self.is_markdown();
        if markdown {
            self.markdown_preview_open = !self.markdown_preview_open;
        }
        markdown
    }

    pub fn is_diff(&self) -> bool {
        matches!(self.kind, EditorKind::Diff(_))
    }

    pub fn diff(&self) -> Option<&DiffView> {
        self.kind.as_diff()
    }

    /// Largest allowed `top_line`: keeps at least one buffer line in view.
    pub fn max_top_line(&self) -> usize {
        self.buffer.line_count().saturating_sub(1)
    }

    /// Vertical scroll position as a `0..=1` fraction of the scrollable range.
    pub fn scroll_fraction(&self) -> f32 {
        match self.max_top_line() {
            0 => 0.0,
            max => self.top_line as f32 / max as f32,
        }
    }

    /// Paint viewport row `row` from the buffer line it shows, with
    /// every decoration this pane carries.
    ///
    /// Fields are destructured so the syntax cache can be borrowed
    /// mutably (it extends itself) while the grid is written and the
    /// buffer is read — three disjoint fields of the same struct.
    fn paint_row(&mut self, row: usize) {
        let Self {
            grid,
            buffer,
            syntax,
            colors,
            find,
            kind,
            top_line,
            h_offset,
            ..
        } = self;
        let line_idx = *top_line + row;
        let cursor_line = buffer.cursor().line;
        let selection = buffer.selection();
        let gutter_w = match kind.as_diff() {
            Some(view) => view.gutter_width(),
            None => grid::gutter_width(buffer.line_count()),
        };

        // Diff decorations first: they decide the gutter text, the tint
        // and whether the row is source code at all.
        let mut gutter_buf = String::new();
        let mut paint = RowPaint::plain(*h_offset, gutter_w, selection, colors);
        if let Some(view) = kind.as_diff() {
            view.gutter_text(line_idx, &mut gutter_buf);
            paint.gutter_text = Some(gutter_buf.as_str());
            paint.gutter_fg = view.gutter_fg(line_idx, colors);
            paint.row_bg = view.row_bg(line_idx, colors);
            paint.fg_override = view.fg_override(line_idx, colors);
            paint.bold = view.is_bold(line_idx);
            let language = view.language_at(line_idx);
            if syntax.language() != language {
                *syntax = SyntaxCache::new(language);
            }
        } else if line_idx == cursor_line && selection.is_none() {
            // The cursor line is only marked when nothing is selected,
            // so the two highlights never fight.
            paint.row_bg = Some(colors.current_line_bg);
        }

        // Diff content rows are tokenized one at a time — each carries
        // its own file's language — but not from a blank slate: the rows
        // inside one hunk are consecutive lines of one file, so the
        // block-comment state entering each of them is precomputed at
        // load and handed in here. Starting every row at `false` painted
        // the second and later lines of a `/* … */` as code.
        let spans_owner;
        if paint.fg_override.is_none() {
            let mut block = kind
                .as_diff()
                .is_some_and(|view| view.block_state_at(line_idx));
            if kind.as_diff().is_some() {
                let mut scratch = Vec::new();
                if syntax.is_active() {
                    crate::syntax::tokenize_spans(
                        buffer.line(line_idx).unwrap_or(""),
                        syntax.language(),
                        &mut block,
                        &mut scratch,
                    );
                }
                spans_owner = scratch;
                paint.spans = &spans_owner;
            } else {
                paint.spans = syntax.spans_for(buffer, line_idx);
            }
        }

        let find_slice;
        if let Some(state) = find.as_ref() {
            find_slice = state.matches_on_line(line_idx).copied().collect::<Vec<_>>();
            paint.finds = &find_slice;
            paint.current_find = state.current_match().filter(|m| m.line == line_idx);
        }

        grid::paint_row(grid, row, line_idx, buffer, &paint);
    }

    fn repaint_viewport(&mut self) {
        for row in 0..self.grid.rows() {
            self.grid.reset_line_identity(row);
            self.paint_row(row);
        }
    }

    fn repaint_visible_row(&mut self, line_idx: usize) {
        if line_idx < self.top_line {
            return;
        }
        let row = line_idx - self.top_line;
        if row >= self.grid.rows() {
            return;
        }
        self.paint_row(row);
    }

    /// Position the grid cursor at the buffer cursor, hiding it when it
    /// is scrolled out of the viewport. Inactive panes additionally hide
    /// it at frame-clone time (main.rs), mirroring terminals.
    fn sync_cursor_into_grid(&mut self) {
        let cursor = self.buffer.cursor();
        let rows = self.grid.rows();
        let cols = self.grid.cols();
        let gutter_w = self.gutter_cells();
        let visual_col = self.buffer.visual_col(cursor);
        let in_vertical = cursor.line >= self.top_line && cursor.line < self.top_line + rows;
        let visible_col = visual_col >= self.h_offset;
        let cell_col = gutter_w + visual_col.saturating_sub(self.h_offset);
        if in_vertical && visible_col && cell_col < cols {
            self.grid.set_cursor(cursor.line - self.top_line, cell_col);
            self.grid.set_cursor_visible(true);
        } else {
            self.grid.set_cursor_visible(false);
        }
    }

    /// Scroll the viewport so `top_line` becomes `target` (clamped).
    /// Returns `true` when the viewport actually moved. Does not move
    /// the cursor (wheel scrolling inspects, it doesn't edit).
    pub fn scroll_to(&mut self, target: usize) -> bool {
        let clamped = target.min(self.max_top_line());
        if clamped == self.top_line {
            return false;
        }
        self.scroll_painted(clamped);
        self.sync_cursor_into_grid();
        true
    }

    /// Move the painted viewport to `top`, reusing the rows that survive
    /// the move so the renderer's line cache stays warm.
    fn scroll_painted(&mut self, top: usize) {
        let plan = grid::plan_scroll(&mut self.grid, self.top_line, top);
        self.top_line = top;
        match plan {
            grid::ScrollPlan::Full => self.repaint_viewport(),
            grid::ScrollPlan::Rows(rows) => {
                for row in rows {
                    self.paint_row(row);
                }
            }
        }
    }

    pub fn scroll_by(&mut self, delta: isize) -> bool {
        let target = self.top_line.saturating_add_signed(delta);
        self.scroll_to(target)
    }

    /// Scroll horizontally by `delta` characters (Shift+wheel). A
    /// changed offset shifts every visible row, so the whole viewport
    /// repaints. Does not move the cursor.
    ///
    /// Clamps against the longest *visible* line — O(rows) per tick,
    /// where a whole-buffer `max_line_chars()` scan would be O(file) on
    /// a wheel-rate hot path.
    pub fn scroll_h_by(&mut self, delta: isize) -> bool {
        let rows = self.grid.rows();
        let max = (self.top_line..self.top_line + rows)
            .map(|l| self.buffer.line_visual_width(l))
            .max()
            .unwrap_or(0)
            .saturating_sub(1);
        let target = self.h_offset.saturating_add_signed(delta).min(max);
        if target == self.h_offset {
            return false;
        }
        self.h_offset = target;
        self.repaint_viewport();
        self.sync_cursor_into_grid();
        true
    }

    /// Gutter width in cells for the current document. Diff panes carry
    /// two number columns and a marker, so their gutter is wider.
    pub fn gutter_cells(&self) -> usize {
        match self.kind.as_diff() {
            Some(view) => view.gutter_width(),
            None => grid::gutter_width(self.buffer.line_count()),
        }
    }

    /// Buffer position rendered at viewport cell (`row`, `col_cell`).
    /// Signed coordinates so drags past the pane edges keep resolving
    /// (negative row maps above the viewport); everything clamps to the
    /// document like code editors do.
    pub fn position_at_cell(&self, row: isize, col_cell: isize) -> Position {
        let last = self.buffer.line_count() as isize - 1;
        let line = (self.top_line as isize + row).clamp(0, last) as usize;
        let content = (col_cell - self.gutter_cells() as isize).max(0) as usize;
        let col = self.buffer.col_for_visual(line, self.h_offset + content);
        Position { line, col }
    }

    /// Resize the viewport grid (e.g. pane layout or font change) and
    /// repaint from the buffer.
    pub fn resize(&mut self, rows: usize, cols: usize) {
        let rows = rows.max(1);
        let cols = cols.max(1);
        if self.grid.rows() == rows && self.grid.cols() == cols {
            return;
        }
        self.grid.resize(rows, cols);
        self.repaint_viewport();
        self.sync_cursor_into_grid();
    }

    /// Record that the buffer's current state is on disk: the dirty
    /// flag clears and future undo steps compare against this point.
    pub fn mark_saved(&mut self) {
        self.buffer.break_undo_group();
        self.saved_top_group = self.buffer.top_group_id();
        self.dirty = false;
    }

    /// Save the buffer to its file atomically (sibling temp file +
    /// rename) with the original line endings. A failed write never
    /// touches the destination. Returns the byte count written.
    ///
    /// Known limitation: renaming over a symlink replaces the link
    /// itself with a regular file rather than writing through to its
    /// target (acceptable for the MVP; symlinked files are rare on
    /// Windows).
    pub fn save(&mut self) -> std::io::Result<u64> {
        // Last line of defence for a diff pane, whose `path` is the
        // repository root: writing there would replace a directory entry
        // with the rendered diff. Callers refuse first; this refuses too.
        if self.read_only {
            return Ok(self.file_bytes);
        }
        let mut text = self.buffer.to_text();
        if self.line_ending == LineEnding::CrLf {
            text = text.replace('\n', "\r\n");
        }
        let bytes = text.into_bytes();
        let file_name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "editor".to_string());
        let tmp =
            self.path
                .with_file_name(format!(".{}.tm-save-{}.tmp", file_name, std::process::id()));
        let write_result = (|| -> std::io::Result<()> {
            use std::io::Write;
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(&bytes)?;
            file.sync_all()
        })();
        if let Err(e) = write_result {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        if let Err(e) = std::fs::rename(&tmp, &self.path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        self.file_bytes = bytes.len() as u64;
        self.mark_saved();
        Ok(bytes.len() as u64)
    }

    /// Scroll (vertically and horizontally) so the cursor is in view.
    /// Returns `true` when the whole viewport was repainted.
    fn ensure_cursor_visible(&mut self) -> bool {
        let cursor = self.buffer.cursor();
        let rows = self.grid.rows();
        let gutter_w = self.gutter_cells();
        let content_cols = self.grid.cols().saturating_sub(gutter_w).max(1);
        let visual_col = self.buffer.visual_col(cursor);

        let mut new_top = self.top_line.min(self.max_top_line());
        if cursor.line < new_top {
            new_top = cursor.line;
        } else if cursor.line >= new_top + rows {
            new_top = cursor.line + 1 - rows;
        }

        let mut new_h = self.h_offset;
        if visual_col < new_h {
            new_h = visual_col;
        } else if visual_col >= new_h + content_cols {
            new_h = visual_col + 1 - content_cols;
        }

        if new_h != self.h_offset {
            // Horizontal scroll invalidates every visible row.
            self.h_offset = new_h;
            self.top_line = new_top;
            self.repaint_viewport();
            return true;
        }
        if new_top != self.top_line {
            self.scroll_painted(new_top);
        }
        false
    }

    /// Run a buffer operation and repaint exactly what it invalidated:
    /// content damage, selection-span changes, scrolling to keep the
    /// cursor visible, and the dirty flag. Returns `true` when anything
    /// changed (callers request a redraw on `true`).
    pub fn apply<F: FnOnce(&mut EditorBuffer) -> buffer::Damage>(&mut self, op: F) -> bool {
        let old_cursor = self.buffer.cursor();
        let old_sel = self.buffer.selection();
        let old_gutter = grid::gutter_width(self.buffer.line_count());

        let damage = op(&mut self.buffer);

        let new_cursor = self.buffer.cursor();
        let new_sel = self.buffer.selection();
        let content_changed = damage != buffer::Damage::None;
        if content_changed {
            self.markdown_cache = OnceLock::new();
            // A read-only pane must never reach here with real damage:
            // `apply_edit` refuses first. Guard anyway so a future caller
            // cannot quietly make a diff pane editable.
            debug_assert!(
                !self.read_only,
                "read-only pane mutated: {}",
                self.display_name
            );
            self.dirty = self.buffer.top_group_id() != self.saved_top_group;
            // Comment state below the edit is no longer trustworthy.
            let first = match damage {
                buffer::Damage::Line(line) | buffer::Damage::From(line) => line,
                buffer::Damage::None => 0,
            };
            self.syntax.invalidate_from(first);
            if let Some(find) = self.find.as_mut() {
                find.refresh(&self.buffer, new_cursor);
            }
        }
        if !content_changed && new_cursor == old_cursor && new_sel == old_sel {
            return false;
        }

        // Deletions can shrink the document above the viewport.
        self.top_line = self.top_line.min(self.max_top_line());

        let repainted_all = if grid::gutter_width(self.buffer.line_count()) != old_gutter {
            // Gutter got wider/narrower: every row's layout shifted.
            self.repaint_viewport();
            self.ensure_cursor_visible();
            true
        } else {
            self.ensure_cursor_visible()
        };

        if !repainted_all {
            let rows = self.grid.rows();
            let last_visible = self.top_line + rows - 1;
            match damage {
                buffer::Damage::From(line) => {
                    for l in line.max(self.top_line)..=last_visible {
                        self.repaint_visible_row(l);
                    }
                }
                buffer::Damage::Line(line) => {
                    // Typing `/` `*` damages one line but changes what
                    // every line under it means. The cache below the edit
                    // was just dropped; without repainting them too the
                    // rows keep their old colours, and scrolling
                    // preserves the stale cells rather than healing it.
                    // Bounded by the viewport, and only for the languages
                    // that have block comments at all.
                    if self.syntax.language().has_block_comments() {
                        for l in line.max(self.top_line)..=last_visible {
                            self.repaint_visible_row(l);
                        }
                    } else {
                        self.repaint_visible_row(line);
                    }
                }
                buffer::Damage::None => {}
            }
            // Repaint lines whose selection membership changed. The
            // union of old and new spans covers grow, shrink, and clear;
            // an empty clamped range simply doesn't iterate.
            if old_sel != new_sel {
                for span in [old_sel, new_sel].into_iter().flatten() {
                    let first = span.0.line.max(self.top_line);
                    let last = span.1.line.min(last_visible);
                    for l in first..=last {
                        self.repaint_visible_row(l);
                    }
                }
            }
            // The cursor-line highlight moves with the cursor, so both
            // the line it left and the line it reached have to be
            // repainted — including when only the selection state
            // changed, which turns the highlight off and on.
            if old_cursor.line != new_cursor.line || old_sel.is_some() != new_sel.is_some() {
                self.repaint_visible_row(old_cursor.line);
                self.repaint_visible_row(new_cursor.line);
            }
        }
        self.sync_cursor_into_grid();
        true
    }

    /// Run a buffer mutation, refusing it on a read-only pane.
    ///
    /// Every editing command goes through here rather than [`apply`],
    /// which stays available for cursor and selection moves (a diff pane
    /// is read-only, not inert — you can still select and copy in it).
    pub fn apply_edit<F: FnOnce(&mut EditorBuffer) -> buffer::Damage>(&mut self, op: F) -> bool {
        if self.read_only {
            return false;
        }
        self.apply(op)
    }

    /// Move the cursor to `pos` and bring it into view, centring the
    /// target when it is outside the current viewport. Used by go-to-
    /// line, find and the Flow hand-offs: landing on the very first or
    /// last row of the pane hides the context that makes a jump useful.
    pub fn reveal(&mut self, pos: Position, extend: bool) -> bool {
        // `set_cursor` clamps the column; the line has to be clamped here
        // too because the scroll target is computed from it.
        let clamped = Position {
            line: pos.line.min(self.max_top_line()),
            col: pos.col,
        };
        let rows = self.grid.rows();
        let visible = clamped.line >= self.top_line && clamped.line < self.top_line + rows;
        if !visible {
            let target = clamped.line.saturating_sub(rows / 2);
            let clamped_top = target.min(self.max_top_line());
            if clamped_top != self.top_line {
                self.scroll_painted(clamped_top);
            }
        }
        self.apply(|b| {
            b.set_cursor(clamped, extend);
            buffer::Damage::None
        })
    }

    /// Jump to a 1-based line (and optional 1-based column), clamped to
    /// the document. Returns the line actually landed on.
    pub fn goto_line(&mut self, line_1: usize, col_1: Option<usize>) -> usize {
        let line = line_1.saturating_sub(1).min(self.max_top_line());
        let col = match col_1 {
            Some(c) => self.buffer.col_for_char_index(line, c.saturating_sub(1)),
            None => 0,
        };
        self.reveal(Position { line, col }, false);
        line + 1
    }

    // -- find bar ---------------------------------------------------------

    /// Open the find bar, seeding it from a single-line selection (the
    /// "search for what I highlighted" reflex) or keeping the previous
    /// query. Returns the query it opened with.
    pub fn find_open(&mut self) -> String {
        let seed = match self.buffer.selection() {
            Some((start, end)) if start.line == end.line && start != end => {
                self.buffer.selected_text()
            }
            _ => None,
        };
        let mut state = self.find.take().unwrap_or_default();
        if let Some(text) = seed {
            state.query = text;
        }
        state.refresh(&self.buffer, self.buffer.cursor());
        self.find = Some(state);
        self.repaint_viewport();
        self.find
            .as_ref()
            .map(|f| f.query.clone())
            .unwrap_or_default()
    }

    /// Close the find bar, keeping the query for the next time it opens.
    /// Returns the closed state for telemetry.
    pub fn find_close(&mut self) -> Option<FindState> {
        let state = self.find.take()?;
        self.repaint_viewport();
        Some(state)
    }

    /// Set the query and re-run the search, keeping the selected match
    /// near the cursor.
    pub fn find_set_query(&mut self, query: &str) -> bool {
        let Some(state) = self.find.as_mut() else {
            return false;
        };
        if state.query == query {
            return false;
        }
        state.query = query.to_string();
        let near = self.buffer.cursor();
        if let Some(state) = self.find.as_mut() {
            state.refresh(&self.buffer, near);
        }
        self.reveal_current_match();
        true
    }

    /// Flip the explicit case-sensitivity toggle.
    pub fn find_toggle_case(&mut self) -> bool {
        let Some(state) = self.find.as_mut() else {
            return false;
        };
        state.case_sensitive = !state.case_sensitive;
        let near = self.buffer.cursor();
        if let Some(state) = self.find.as_mut() {
            state.refresh(&self.buffer, near);
        }
        self.reveal_current_match();
        true
    }

    /// Step to the next (`forward`) or previous match and reveal it.
    pub fn find_step(&mut self, forward: bool) -> bool {
        let Some(state) = self.find.as_mut() else {
            return false;
        };
        let stepped = if forward { state.next() } else { state.prev() };
        if stepped.is_none() {
            return false;
        }
        self.reveal_current_match();
        true
    }

    /// Select the current match and scroll it into view. Always
    /// repaints the viewport: the tint on every other match moves too.
    fn reveal_current_match(&mut self) {
        let Some(found) = self.find.as_ref().and_then(|f| f.current_match()) else {
            self.repaint_viewport();
            return;
        };
        self.reveal(
            Position {
                line: found.line,
                col: found.start,
            },
            false,
        );
        // Select the hit so Ctrl+C copies it and the cursor sits at its
        // start, matching what every editor does on "find next".
        self.apply(|b| {
            b.set_cursor(
                Position {
                    line: found.line,
                    col: found.start,
                },
                false,
            );
            b.set_cursor(
                Position {
                    line: found.line,
                    col: found.end,
                },
                true,
            );
            buffer::Damage::None
        });
        self.repaint_viewport();
    }

    // -- diff navigation --------------------------------------------------

    /// Move the cursor to the next or previous hunk header, scrolled to
    /// the top of the viewport so the whole hunk is visible below it.
    pub fn diff_step_hunk(&mut self, forward: bool) -> bool {
        self.diff_step(forward, true)
    }

    pub fn diff_step_file(&mut self, forward: bool) -> bool {
        self.diff_step(forward, false)
    }

    fn diff_step(&mut self, forward: bool, hunk: bool) -> bool {
        let from = self.buffer.cursor().line;
        let Some(view) = self.kind.as_diff() else {
            return false;
        };
        let target = match (hunk, forward) {
            (true, true) => view.next_hunk(from),
            (true, false) => view.prev_hunk(from),
            (false, true) => view.next_file(from),
            (false, false) => view.prev_file(from),
        };
        let Some(target) = target else {
            return false;
        };
        // A header at the top of the viewport shows its hunk; centring it
        // would waste half the pane on the previous hunk's tail.
        let top = target.min(self.max_top_line());
        if top != self.top_line {
            self.scroll_painted(top);
        }
        self.apply(|b| {
            b.set_cursor(
                Position {
                    line: target,
                    col: 0,
                },
                false,
            );
            buffer::Damage::None
        });
        true
    }

    /// Scroll a diff pane to `rel`'s section, and to `line` within it
    /// when the diff contains that line. Used by the Flow hand-off.
    pub fn diff_reveal_location(&mut self, rel: &str, line: Option<u32>) -> bool {
        let target = {
            let Some(view) = self.kind.as_diff() else {
                return false;
            };
            match line {
                Some(line) => view.find_line_row(rel, line),
                None => view.find_file_row(rel),
            }
        };
        let Some(target) = target else {
            return false;
        };
        let top = target.min(self.max_top_line());
        if top != self.top_line {
            self.scroll_painted(top);
        }
        self.apply(|b| {
            b.set_cursor(
                Position {
                    line: target,
                    col: 0,
                },
                false,
            );
            buffer::Damage::None
        });
        true
    }

    /// The file and 1-based new-file line under the cursor of a diff
    /// pane, for "open this in an editor".
    pub fn diff_open_target(&self) -> Option<(PathBuf, u32)> {
        let view = self.kind.as_diff()?;
        let (file, line) = view.open_target(self.buffer.cursor().line)?;
        if file.status == crate::diff::DiffFileStatus::Deleted {
            return None;
        }
        // Never `join`: `file.path` came out of git's own output.
        Some((
            crate::git::resolve_in_repo(&view.repo_root, &file.path)?,
            line,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A `.rs` file, so the pane opens with Rust highlighting on.
    fn temp_rust_file(contents: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tm-editor-rs-{}-{}.rs",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, contents).expect("write temp file");
        path
    }

    /// Foreground of the first non-blank cell of `row`, i.e. the colour
    /// of the row's first token once the gutter is past.
    fn first_text_fg(pane: &EditorPane, row: usize) -> Option<unshit::core::style::types::Color> {
        (pane.gutter_cells()..pane.grid.cols()).find_map(|col| {
            let cell = pane.grid.get_cell(row, col)?;
            (cell.ch != '\0' && cell.ch != ' ').then_some(cell.fg)
        })
    }

    /// Typing `/` `*` damages one line and changes what every line below
    /// it means. Repainting only the damaged row left the rest of the
    /// viewport coloured as code, and scrolling preserved the stale cells
    /// rather than healing it.
    #[test]
    fn opening_a_block_comment_recolours_the_rows_below_it() {
        let path = temp_rust_file("let a = 1;\nlet b = 2;\nlet c = 3;\n");
        let mut pane = EditorPane::open(&path, 8, 40).expect("open");
        let colors = pane.colors;

        let before = first_text_fg(&pane, 1).expect("row 1 painted");
        assert_eq!(before, colors.keyword, "`let` starts out a keyword");

        // Open a block comment at the very top of the file.
        pane.apply_edit(|b| {
            b.set_cursor(Position { line: 0, col: 0 }, false);
            b.insert_str("/*")
        });

        assert_eq!(
            first_text_fg(&pane, 1),
            Some(colors.comment),
            "row 1 is inside the comment now"
        );
        assert_eq!(
            first_text_fg(&pane, 2),
            Some(colors.comment),
            "and so is row 2"
        );

        // Closing it again puts them back.
        pane.apply_edit(|b| {
            b.set_cursor(Position { line: 0, col: 2 }, false);
            b.insert_str("*/")
        });
        assert_eq!(first_text_fg(&pane, 1), Some(colors.keyword));
        assert_eq!(first_text_fg(&pane, 2), Some(colors.keyword));

        let _ = std::fs::remove_file(path);
    }

    fn temp_file(contents: &[u8]) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tm-editor-open-{}-{}.txt",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let mut f = std::fs::File::create(&path).expect("create temp file");
        f.write_all(contents).expect("write temp file");
        path
    }

    #[test]
    fn markdown_preview_document_follows_buffer_edits() {
        let path = std::env::temp_dir().join(format!(
            "tm-editor-markdown-{}-{}.md",
            std::process::id(),
            generate_correlation_id()
        ));
        std::fs::write(&path, "# Before\n").expect("write temp file");
        let mut pane = EditorPane::open(&path, 10, 40).expect("open");
        let before = pane.markdown_document().expect("markdown document");

        assert!(pane.apply_edit(|buffer| {
            buffer.set_cursor(Position { line: 0, col: 8 }, false);
            buffer.insert_str("After")
        }));
        let after = pane.markdown_document().expect("updated markdown document");

        assert_ne!(before.blocks, after.blocks);
        assert_eq!(
            after.blocks.first(),
            Some(&crate::markdown::MarkdownBlock::Heading {
                level: 1,
                text: "BeforeAfter".into(),
                source_line: 0,
                anchor: "beforeafter".into()
            })
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn open_reads_lf_file() {
        let path = temp_file(b"alpha\nbeta\n");
        let pane = EditorPane::open(&path, 10, 40).expect("open");
        assert_eq!(pane.buffer.line_count(), 3);
        assert_eq!(pane.line_ending, LineEnding::Lf);
        assert!(!pane.dirty);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn open_normalizes_crlf_and_remembers_it() {
        let path = temp_file(b"alpha\r\nbeta");
        let pane = EditorPane::open(&path, 10, 40).expect("open");
        assert_eq!(pane.line_ending, LineEnding::CrLf);
        assert_eq!(pane.buffer.line(0), Some("alpha"));
        assert_eq!(pane.buffer.line(1), Some("beta"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn open_normalizes_lone_cr_line_endings() {
        // Classic-Mac file: bare CR separators, no LF anywhere.
        let path = temp_file(b"alpha\rbeta");
        let pane = EditorPane::open(&path, 10, 40).expect("open");
        assert_eq!(pane.buffer.line(0), Some("alpha"));
        assert_eq!(pane.buffer.line(1), Some("beta"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn open_normalizes_stray_cr_in_crlf_file() {
        // Mixed endings: CRLF-dominant with one bare CR. No line may
        // keep a literal \r (the buffer invariant every consumer
        // relies on).
        let path = temp_file(b"a\r\nb\rc");
        let pane = EditorPane::open(&path, 10, 40).expect("open");
        assert_eq!(pane.line_ending, LineEnding::CrLf);
        for i in 0..pane.buffer.line_count() {
            let line = pane.buffer.line(i).unwrap();
            assert!(!line.contains('\r'), "line {i} kept a \\r: {line:?}");
        }
        assert_eq!(pane.buffer.line_count(), 3);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn open_refuses_invalid_utf8() {
        let path = temp_file(&[0xff, 0xfe, 0x00, 0x41]);
        match EditorPane::open(&path, 10, 40) {
            Err(OpenError::InvalidUtf8) => {}
            other => panic!("expected InvalidUtf8, got {:?}", other.err()),
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn open_refuses_missing_file() {
        let path = std::env::temp_dir().join("tm-editor-definitely-missing.txt");
        assert!(matches!(
            EditorPane::open(&path, 10, 40),
            Err(OpenError::Io(_))
        ));
    }

    #[test]
    fn scroll_clamps_to_document() {
        let path = temp_file(b"a\nb\nc\nd\ne");
        let mut pane = EditorPane::open(&path, 3, 20).expect("open");
        assert!(pane.scroll_by(100));
        assert_eq!(pane.top_line, 4);
        assert!(!pane.scroll_by(1));
        assert!(pane.scroll_by(-100));
        assert_eq!(pane.top_line, 0);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn position_at_cell_maps_content_cells_to_buffer_positions() {
        let path = temp_file(b"hello\nworld wide");
        let pane = EditorPane::open(&path, 5, 40).expect("open");
        let gutter = pane.gutter_cells() as isize;
        // Row 1, third character of "world wide".
        let pos = pane.position_at_cell(1, gutter + 2);
        assert_eq!(pos, Position { line: 1, col: 2 });
        // Clicks inside the gutter clamp to column 0.
        assert_eq!(pane.position_at_cell(0, 1), Position { line: 0, col: 0 });
        // Past the end of the line clamps to line end.
        let pos = pane.position_at_cell(0, gutter + 99);
        assert_eq!(pos, Position { line: 0, col: 5 });
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn position_at_cell_clamps_rows_to_document() {
        let path = temp_file(b"a\nb\nc\nd\ne\nf");
        let mut pane = EditorPane::open(&path, 3, 20).expect("open");
        pane.scroll_to(2);
        // Negative row (drag above the pane) resolves above the viewport.
        assert_eq!(pane.position_at_cell(-1, 5).line, 1);
        // Far below the last line clamps to the last line.
        assert_eq!(pane.position_at_cell(99, 5).line, 5);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn position_at_cell_respects_h_offset() {
        let path = temp_file(b"abcdefghij");
        let mut pane = EditorPane::open(&path, 2, 20).expect("open");
        assert!(pane.scroll_h_by(3));
        let gutter = pane.gutter_cells() as isize;
        // First content cell now renders 'd' (index 3).
        assert_eq!(
            pane.position_at_cell(0, gutter),
            Position { line: 0, col: 3 }
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn scroll_h_clamps_and_repaints() {
        let path = temp_file(b"abcdefghij\nxy");
        let mut pane = EditorPane::open(&path, 2, 8).expect("open");
        assert!(pane.scroll_h_by(100));
        assert_eq!(pane.h_offset, 9, "clamps to longest line minus one");
        let gutter = pane.gutter_cells();
        assert_eq!(
            pane.grid.get_cell(0, gutter).map(|c| c.ch),
            Some('j'),
            "viewport repaints from the new offset"
        );
        assert!(pane.scroll_h_by(-100));
        assert_eq!(pane.h_offset, 0);
        assert!(!pane.scroll_h_by(-1), "already at the left edge");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn resize_repaints_viewport() {
        let path = temp_file(b"one\ntwo\nthree\nfour");
        let mut pane = EditorPane::open(&path, 2, 10).expect("open");
        pane.resize(4, 30);
        assert_eq!(pane.grid.rows(), 4);
        assert_eq!(pane.grid.cols(), 30);
        let row3: String = (0..pane.grid.cols())
            .filter_map(|c| pane.grid.get_cell(3, c).map(|cell| cell.ch))
            .filter(|&ch| ch != '\0')
            .collect();
        assert!(row3.contains("four"));
        let _ = std::fs::remove_file(path);
    }
}
