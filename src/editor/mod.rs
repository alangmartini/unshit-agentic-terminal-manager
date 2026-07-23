//! Built-in file editor: pane model, file open/save, viewport state.
//!
//! Editor panes live beside terminal panes in the tab/split layout. The
//! pane owns a pure `EditorBuffer` plus a live `CellGrid` painted with
//! the visible window only, mirroring how terminals publish their
//! display grid — the render tree never sees the whole file.

pub mod buffer;
pub mod grid;
pub mod telemetry;

use std::path::{Path, PathBuf};

use unshit::core::cell_grid::CellGrid;

pub use buffer::{Damage, EditorBuffer, LineEnding, Position, TAB_SPACES};

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
    let normalized = if line_ending == LineEnding::CrLf {
        text.replace("\r\n", "\n")
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
    /// Characters skipped at the left of every line (horizontal scroll).
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
}

impl EditorPane {
    pub fn open(path: &Path, rows: usize, cols: usize) -> Result<Self, OpenError> {
        let (buffer, line_ending, file_bytes) = load_file(path)?;
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
        };
        pane.repaint_viewport();
        pane.sync_cursor_into_grid();
        Ok(pane)
    }

    /// Largest allowed `top_line`: keeps at least one buffer line in view.
    pub fn max_top_line(&self) -> usize {
        self.buffer.line_count().saturating_sub(1)
    }

    fn repaint_viewport(&mut self) {
        grid::repaint_all(
            &mut self.grid,
            &self.buffer,
            self.top_line,
            self.h_offset,
            self.buffer.selection(),
        );
    }

    fn repaint_visible_row(&mut self, line_idx: usize) {
        if line_idx < self.top_line {
            return;
        }
        let row = line_idx - self.top_line;
        if row >= self.grid.rows() {
            return;
        }
        let gutter_w = grid::gutter_width(self.buffer.line_count());
        let selection = self.buffer.selection();
        grid::paint_row(
            &mut self.grid,
            row,
            line_idx,
            &self.buffer,
            self.h_offset,
            gutter_w,
            selection,
        );
    }

    /// Position the grid cursor at the buffer cursor, hiding it when it
    /// is scrolled out of the viewport. Inactive panes additionally hide
    /// it at frame-clone time (main.rs), mirroring terminals.
    fn sync_cursor_into_grid(&mut self) {
        let cursor = self.buffer.cursor();
        let rows = self.grid.rows();
        let cols = self.grid.cols();
        let gutter_w = grid::gutter_width(self.buffer.line_count());
        let char_col = self.buffer.char_col(cursor);
        let in_vertical = cursor.line >= self.top_line && cursor.line < self.top_line + rows;
        let visible_col = char_col >= self.h_offset;
        let cell_col = gutter_w + char_col.saturating_sub(self.h_offset);
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
        grid::scroll_viewport(
            &mut self.grid,
            &self.buffer,
            self.top_line,
            clamped,
            self.h_offset,
            self.buffer.selection(),
        );
        self.top_line = clamped;
        self.sync_cursor_into_grid();
        true
    }

    pub fn scroll_by(&mut self, delta: isize) -> bool {
        let target = self.top_line.saturating_add_signed(delta);
        self.scroll_to(target)
    }

    /// Scroll horizontally by `delta` characters (Shift+wheel). A
    /// changed offset shifts every visible row, so the whole viewport
    /// repaints. Does not move the cursor.
    pub fn scroll_h_by(&mut self, delta: isize) -> bool {
        let max = self.buffer.max_line_chars().saturating_sub(1);
        let target = self.h_offset.saturating_add_signed(delta).min(max);
        if target == self.h_offset {
            return false;
        }
        self.h_offset = target;
        self.repaint_viewport();
        self.sync_cursor_into_grid();
        true
    }

    /// Gutter width in cells for the current document.
    pub fn gutter_cells(&self) -> usize {
        grid::gutter_width(self.buffer.line_count())
    }

    /// Buffer position rendered at viewport cell (`row`, `col_cell`).
    /// Signed coordinates so drags past the pane edges keep resolving
    /// (negative row maps above the viewport); everything clamps to the
    /// document like code editors do.
    pub fn position_at_cell(&self, row: isize, col_cell: isize) -> Position {
        let last = self.buffer.line_count() as isize - 1;
        let line = (self.top_line as isize + row).clamp(0, last) as usize;
        let content = (col_cell - self.gutter_cells() as isize).max(0) as usize;
        let col = self
            .buffer
            .col_for_char_index(line, self.h_offset + content);
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
    pub fn save(&mut self) -> std::io::Result<u64> {
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
        let gutter_w = grid::gutter_width(self.buffer.line_count());
        let content_cols = self.grid.cols().saturating_sub(gutter_w).max(1);
        let char_col = self.buffer.char_col(cursor);

        let mut new_top = self.top_line.min(self.max_top_line());
        if cursor.line < new_top {
            new_top = cursor.line;
        } else if cursor.line >= new_top + rows {
            new_top = cursor.line + 1 - rows;
        }

        let mut new_h = self.h_offset;
        if char_col < new_h {
            new_h = char_col;
        } else if char_col >= new_h + content_cols {
            new_h = char_col + 1 - content_cols;
        }

        if new_h != self.h_offset {
            // Horizontal scroll invalidates every visible row.
            self.h_offset = new_h;
            self.top_line = new_top;
            self.repaint_viewport();
            return true;
        }
        if new_top != self.top_line {
            grid::scroll_viewport(
                &mut self.grid,
                &self.buffer,
                self.top_line,
                new_top,
                self.h_offset,
                self.buffer.selection(),
            );
            self.top_line = new_top;
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
            self.dirty = self.buffer.top_group_id() != self.saved_top_group;
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
                buffer::Damage::Line(line) => self.repaint_visible_row(line),
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
        }
        self.sync_cursor_into_grid();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

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
