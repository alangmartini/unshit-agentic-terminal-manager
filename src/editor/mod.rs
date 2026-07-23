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

pub use buffer::{EditorBuffer, LineEnding};

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
        let mut grid = CellGrid::new(rows.max(1), cols.max(1));
        grid.set_cursor_visible(false);
        grid::repaint_all(&mut grid, &buffer, 0, 0);
        Ok(Self {
            path: path.to_path_buf(),
            display_name: display_name(path),
            buffer,
            top_line: 0,
            h_offset: 0,
            dirty: false,
            correlation_id: generate_correlation_id(),
            line_ending,
            file_bytes,
            grid,
        })
    }

    /// Largest allowed `top_line`: keeps at least one buffer line in view.
    pub fn max_top_line(&self) -> usize {
        self.buffer.line_count().saturating_sub(1)
    }

    /// Scroll the viewport so `top_line` becomes `target` (clamped).
    /// Returns `true` when the viewport actually moved.
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
        );
        self.top_line = clamped;
        true
    }

    pub fn scroll_by(&mut self, delta: isize) -> bool {
        let target = self.top_line.saturating_add_signed(delta);
        self.scroll_to(target)
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
        self.grid.set_cursor_visible(false);
        grid::repaint_all(&mut self.grid, &self.buffer, self.top_line, self.h_offset);
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
