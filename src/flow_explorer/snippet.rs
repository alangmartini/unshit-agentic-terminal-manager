//! Source excerpts for a node's location, read from the flow's repo root.
//!
//! Loading happens on the dispatch path (`flow.src:` / column focus), never
//! during render, and results (including failures) are cached per node so a
//! missing file is not re-stat'd every frame.

use std::path::{Path, PathBuf};

use super::highlight::Language;
use super::model::{is_safe_relative_path, Location};

/// Refuse to read files above this size; a snippet is a few lines and the
/// editor applies the same kind of guard.
pub const MAX_SNIPPET_BYTES: u64 = 256 * 1024;
/// Lines shown above and below the located range.
pub const CONTEXT_LINES: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnippetError {
    /// The location's path is absolute, escapes the root, or resolves
    /// outside it after canonicalisation.
    UnsafePath,
    /// `repo_root` is not a directory (the flow was authored elsewhere).
    RepoRootMissing,
    NotFound,
    TooLarge(u64),
    Io(String),
    InvalidUtf8,
}

impl SnippetError {
    /// Bounded, label-safe reason for telemetry.
    pub fn reason(&self) -> &'static str {
        match self {
            SnippetError::UnsafePath => "unsafe_path",
            SnippetError::RepoRootMissing => "repo_root_missing",
            SnippetError::NotFound => "not_found",
            SnippetError::TooLarge(_) => "too_large",
            SnippetError::Io(_) => "io",
            SnippetError::InvalidUtf8 => "invalid_utf8",
        }
    }

    /// Stale flows (branch switched, file moved) are expected and render
    /// inline without telemetry; the rest are worth a log line.
    pub fn is_expected(&self) -> bool {
        matches!(self, SnippetError::NotFound | SnippetError::RepoRootMissing)
    }

    /// Short text rendered in place of the snippet.
    pub fn message(&self) -> String {
        match self {
            SnippetError::UnsafePath => "source path is outside the repo".to_string(),
            SnippetError::RepoRootMissing => "repo root not found".to_string(),
            SnippetError::NotFound => "source not available".to_string(),
            SnippetError::TooLarge(len) => format!("file too large ({} KiB)", len / 1024),
            SnippetError::Io(err) => format!("could not read source: {err}"),
            SnippetError::InvalidUtf8 => "source is not UTF-8".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    /// 1-based number of `lines[0]`.
    pub first_line: u32,
    pub lines: Vec<String>,
    /// Inclusive 1-based range the location points at (clamped to the file).
    pub hl_start: u32,
    pub hl_end: u32,
    pub language: Language,
    /// The location's repo-relative file, as authored in the flow.
    pub file: String,
    /// The file as resolved on disk.
    pub path: PathBuf,
}

impl Snippet {
    /// Whether the 1-based line number is inside the located range.
    pub fn is_highlighted(&self, line_no: u32) -> bool {
        (self.hl_start..=self.hl_end).contains(&line_no)
    }

    /// Digits needed for the gutter (at least 3, like the editor).
    pub fn gutter_width(&self) -> usize {
        let last = self.first_line as usize + self.lines.len().saturating_sub(1);
        last.to_string().len().max(3)
    }
}

/// Read `location` from `repo_root` with `context` lines either side.
pub fn load_snippet(
    repo_root: &Path,
    location: &Location,
    context: u32,
) -> Result<Snippet, SnippetError> {
    if !is_safe_relative_path(&location.file) {
        return Err(SnippetError::UnsafePath);
    }
    let root = std::fs::canonicalize(repo_root).map_err(|_| SnippetError::RepoRootMissing)?;
    if !root.is_dir() {
        return Err(SnippetError::RepoRootMissing);
    }
    let candidate = root.join(location.file.replace('/', std::path::MAIN_SEPARATOR_STR));
    let meta = match std::fs::metadata(&candidate) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(SnippetError::NotFound)
        }
        Err(err) => return Err(SnippetError::Io(err.to_string())),
    };
    if !meta.is_file() {
        return Err(SnippetError::NotFound);
    }
    if meta.len() > MAX_SNIPPET_BYTES {
        return Err(SnippetError::TooLarge(meta.len()));
    }
    // Re-check containment on the resolved path: a symlink or junction
    // inside the repo may still point outside it.
    let resolved =
        std::fs::canonicalize(&candidate).map_err(|e| SnippetError::Io(e.to_string()))?;
    if !resolved.starts_with(&root) {
        return Err(SnippetError::UnsafePath);
    }
    let bytes = std::fs::read(&resolved).map_err(|e| SnippetError::Io(e.to_string()))?;
    let text = String::from_utf8(bytes).map_err(|_| SnippetError::InvalidUtf8)?;
    let all: Vec<&str> = text.lines().collect();
    let total = all.len() as u32;
    if total == 0 {
        return Err(SnippetError::NotFound);
    }

    let start_target = location.line.max(1);
    let end_target = location.end_line.unwrap_or(start_target).max(start_target);
    let hl_start = start_target.min(total);
    let hl_end = end_target.min(total);
    let first_line = hl_start.saturating_sub(context).max(1);
    let last_line = hl_end.saturating_add(context).min(total);
    let lines = all[(first_line - 1) as usize..last_line as usize]
        .iter()
        .map(|l| l.trim_end_matches('\r').to_string())
        .collect();

    Ok(Snippet {
        first_line,
        lines,
        hl_start,
        hl_end,
        language: Language::from_path(&location.file),
        file: location.file.clone(),
        path: resolved,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_explorer::resolve_repo_root;
    use crate::flow_explorer::test_support::{fixture_path, load_fixture};

    fn root() -> PathBuf {
        let flow = load_fixture();
        resolve_repo_root(&fixture_path(), &flow.repo_root)
    }

    fn loc(file: &str, line: u32, end: Option<u32>) -> Location {
        Location {
            file: file.into(),
            line,
            end_line: end,
        }
    }

    #[test]
    fn loads_range_with_context_and_highlights() {
        let s = load_snippet(
            &root(),
            &loc(
                "packages/server/src/sessions/SessionRegistry.ts",
                31,
                Some(41),
            ),
            3,
        )
        .unwrap();
        assert_eq!(s.first_line, 28);
        assert_eq!(s.hl_start, 31);
        assert_eq!(s.hl_end, 41);
        assert!(s.lines.len() >= 14, "{}", s.lines.len());
        assert!(s.is_highlighted(31));
        assert!(s.is_highlighted(41));
        assert!(!s.is_highlighted(30));
        assert_eq!(s.language, Language::TypeScript);
        assert_eq!(s.file, "packages/server/src/sessions/SessionRegistry.ts");
        assert!(
            s.path.ends_with("SessionRegistry.ts"),
            "{}",
            s.path.display()
        );
        assert_eq!(s.gutter_width(), 3);
        assert!(s.lines.iter().any(|l| l.contains("open")));
    }

    #[test]
    fn clamps_to_file_start_and_end() {
        let s = load_snippet(
            &root(),
            &loc(
                "apps/electron/src/renderer/main/agent/Editor.tsx",
                1,
                Some(2),
            ),
            3,
        )
        .unwrap();
        assert_eq!(s.first_line, 1);
        assert_eq!(s.hl_start, 1);
        let s = load_snippet(
            &root(),
            &loc(
                "apps/electron/src/renderer/main/agent/Editor.tsx",
                9_000,
                None,
            ),
            3,
        )
        .unwrap();
        assert_eq!(s.hl_start, s.hl_end);
        assert_eq!(s.first_line + s.lines.len() as u32 - 1, s.hl_end);
    }

    #[test]
    fn missing_file_is_expected_not_found() {
        let err = load_snippet(&root(), &loc("nope/Missing.ts", 1, None), 3).unwrap_err();
        assert_eq!(err, SnippetError::NotFound);
        assert!(err.is_expected());
        assert_eq!(err.reason(), "not_found");
    }

    #[test]
    fn missing_root_is_expected() {
        let err = load_snippet(
            Path::new("C:/definitely/not/here"),
            &loc("a.ts", 1, None),
            3,
        )
        .unwrap_err();
        assert_eq!(err, SnippetError::RepoRootMissing);
        assert!(err.is_expected());
    }

    #[test]
    fn rejects_unsafe_paths() {
        for file in ["../Cargo.toml", "/etc/passwd", "C:/Windows/win.ini", ""] {
            let err = load_snippet(&root(), &loc(file, 1, None), 3).unwrap_err();
            assert_eq!(err, SnippetError::UnsafePath, "{file}");
            assert!(!err.is_expected());
        }
    }

    #[test]
    fn rejects_files_over_the_cap() {
        let dir = std::env::temp_dir().join(format!("flow-snippet-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let big = dir.join("big.rs");
        std::fs::write(&big, vec![b'x'; MAX_SNIPPET_BYTES as usize + 1]).unwrap();
        let err = load_snippet(&dir, &loc("big.rs", 1, None), 3).unwrap_err();
        assert!(matches!(err, SnippetError::TooLarge(_)), "{err:?}");
        assert_eq!(err.reason(), "too_large");
        std::fs::write(&big, [0xff, 0xfe, 0x00, 0x41]).unwrap();
        let err = load_snippet(&dir, &loc("big.rs", 1, None), 3).unwrap_err();
        assert_eq!(err, SnippetError::InvalidUtf8);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
