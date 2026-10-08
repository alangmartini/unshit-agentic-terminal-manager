//! Classify filesystem paths delivered by Finder, Explorer, and the CLI.
//!
//! Platform integrations must keep paths as `OsString`/`PathBuf` values until
//! after filesystem resolution. In particular, converting a Windows or Unix
//! path to UTF-8 before opening it would break valid filenames.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// A path the desktop shell asked Terminal Manager to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchTarget {
    /// Start a terminal whose working directory is this folder.
    Folder(PathBuf),
    /// Open a text or Markdown file in the built-in editor.
    TextFile(PathBuf),
    /// Open a unified diff in the patch review surface.
    PatchFile(PathBuf),
}

impl LaunchTarget {
    pub fn path(&self) -> &Path {
        match self {
            Self::Folder(path) | Self::TextFile(path) | Self::PatchFile(path) => path,
        }
    }

    /// The workspace root that should be selected before handling this target.
    pub fn workspace_root(&self) -> Option<PathBuf> {
        match self {
            Self::Folder(path) => Some(path.clone()),
            Self::TextFile(path) | Self::PatchFile(path) => path.parent().map(Path::to_path_buf),
        }
    }
}

/// The result of parsing process arguments that may represent a desktop-shell
/// open request. Other product CLI commands deliberately remain untouched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseResult {
    NotRequested,
    Target(LaunchTarget),
    Error(String),
}

/// Parse a desktop open request from process arguments using the process CWD
/// for relative input paths.
pub fn parse_process_args<I, S>(args: I) -> ParseResult
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(error) => {
            return ParseResult::Error(format!("could not resolve current directory: {error}"))
        }
    };
    parse_args_from_dir(args, &cwd)
}

/// Parse a desktop open request with an explicit base directory. Kept public
/// for deterministic tests and for host integrations that receive relative
/// paths from an external process.
pub fn parse_args_from_dir<I, S>(args: I, cwd: &Path) -> ParseResult
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let Some(first) = args.first() else {
        return ParseResult::NotRequested;
    };

    let command = first.to_str();
    let (path, expected) = match command {
        Some("open") => match exactly_one_path(&args[1..], "open") {
            Ok(path) => (path, ExpectedKind::Any),
            Err(error) => return ParseResult::Error(error),
        },
        Some("open-folder") | Some("open-terminal") | Some("open-terminal-here") => {
            match exactly_one_path(&args[1..], "open-folder") {
                Ok(path) => (path, ExpectedKind::Folder),
                Err(error) => return ParseResult::Error(error),
            }
        }
        Some("open-file") => match exactly_one_path(&args[1..], "open-file") {
            Ok(path) => (path, ExpectedKind::File),
            Err(error) => return ParseResult::Error(error),
        },
        Some("open-patch") => match exactly_one_path(&args[1..], "open-patch") {
            Ok(path) => (path, ExpectedKind::Patch),
            Err(error) => return ParseResult::Error(error),
        },
        // Finder can launch an application with the selected path as its
        // sole positional argument. Support that form too, while leaving the
        // existing named CLI commands (agent, flow, notify, ...) alone.
        //
        // This check must happen before resolving the path: commands such as
        // `terminal-manager agent` deliberately have no positional target,
        // and must reach the notification CLI parser in `main` unchanged.
        _ if args.len() == 1
            && !first.to_string_lossy().starts_with('-')
            && !is_notification_cli_command(first) =>
        {
            (PathBuf::from(first), ExpectedKind::Any)
        }
        _ => return ParseResult::NotRequested,
    };

    match resolve_target_from_dir(&path, cwd) {
        Ok(target) if expected.accepts(&target) => ParseResult::Target(target),
        Ok(_) => ParseResult::Error(expected.error_message()),
        Err(error) => ParseResult::Error(error),
    }
}

/// Keep names owned by the notification/agent CLI out of Finder's
/// raw-positional fallback so their own parser can emit the correct behavior
/// and diagnostics. Defers to [`crate::notifications::is_top_level_cli_command`]
/// so this list cannot drift from the one the CLI parser actually dispatches on.
fn is_notification_cli_command(arg: &OsStr) -> bool {
    arg.to_str()
        .is_some_and(crate::notifications::is_top_level_cli_command)
}

/// Validate an absolute filesystem path received over local IPC or from an
/// OS-native open-files callback.
pub fn resolve_absolute_target(path: &Path) -> Result<LaunchTarget, String> {
    if !path.is_absolute() {
        return Err("desktop open paths must be absolute".to_string());
    }
    resolve_target(path)
}

/// Classify a supported document from its filename extension without touching
/// the filesystem. The in-app Explorer uses this while building a context
/// menu, where a synchronous metadata read would be inappropriate.
pub fn classify_supported_file_path(path: &Path) -> Option<LaunchTarget> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase());
    match extension.as_deref() {
        Some("txt") | Some("md") | Some("markdown") => Some(LaunchTarget::TextFile(path.into())),
        Some("patch") | Some("diff") => Some(LaunchTarget::PatchFile(path.into())),
        _ => None,
    }
}

/// Whether two paths name the same location, compared without touching the
/// filesystem.
///
/// Callers run this while holding the app state lock, so it must never block:
/// a workspace on an unreachable network share or a stopped WSL distro made a
/// per-workspace `canonicalize` hang the UI. On Windows the comparison also
/// ignores the `\\?\` prefix `canonicalize` adds, separator style, and case,
/// which is how the filesystem itself compares names there.
pub fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    #[cfg(windows)]
    if let (Some(a), Some(b)) = (a.to_str(), b.to_str()) {
        return windows_path_key(a) == windows_path_key(b);
    }
    false
}

/// Canonicalize `path`, then simplify away Windows' `\\?\` prefix in one
/// atomic step. Callers that store or display the result must go through
/// this rather than `Path::canonicalize` directly, since a second
/// `canonicalize` of an already-simplified path re-adds the prefix.
pub fn canonicalize_plain(path: &Path) -> std::io::Result<PathBuf> {
    path.canonicalize().map(|path| simplify_verbatim(&path))
}

/// Drop the `\\?\` prefix Windows' `canonicalize` adds when the plain form
/// names the same file. Stored verbatim, the prefix leaks into workspace
/// paths, terminal working directories, and every later path comparison.
fn simplify_verbatim(path: &Path) -> PathBuf {
    #[cfg(windows)]
    if let Some(simplified) = path.to_str().and_then(simplify_verbatim_str) {
        return PathBuf::from(simplified);
    }
    path.to_path_buf()
}

/// `\\?\C:\dir` -> `C:\dir` and `\\?\UNC\server\share\dir` ->
/// `\\server\share\dir`. `None` when the path is not verbatim or when Win32
/// normalization would change what the plain form refers to (over-long
/// paths, reserved device names, components ending in a dot or space).
#[cfg_attr(not(windows), allow(dead_code))]
fn simplify_verbatim_str(path: &str) -> Option<String> {
    const MAX_PATH: usize = 260;
    let rest = path.strip_prefix(r"\\?\")?;
    let simplified = if let Some(unc) = rest.strip_prefix(r"UNC\") {
        format!(r"\\{unc}")
    } else {
        let bytes = rest.as_bytes();
        let is_drive_root = bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'\\';
        if !is_drive_root {
            return None;
        }
        rest.to_string()
    };
    if simplified.len() >= MAX_PATH || simplified.contains('/') {
        return None;
    }
    let components = simplified.split('\\').skip(1).filter(|c| !c.is_empty());
    for component in components {
        if component == "."
            || component == ".."
            || component.ends_with('.')
            || component.ends_with(' ')
            || is_reserved_device_name(component)
        {
            return None;
        }
    }
    Some(simplified)
}

#[cfg_attr(not(windows), allow(dead_code))]
fn is_reserved_device_name(component: &str) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or(component)
        .trim_end()
        .to_ascii_uppercase();
    match stem.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" => true,
        _ => {
            let bytes = stem.as_bytes();
            bytes.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && (b'1'..=b'9').contains(&bytes[3])
        }
    }
}

/// Comparison key for a Windows path string: verbatim prefix dropped,
/// separators unified, trailing separators trimmed, case folded.
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_path_key(path: &str) -> String {
    let plain = simplify_verbatim_str(path).unwrap_or_else(|| path.to_string());
    plain
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

fn exactly_one_path(args: &[OsString], command: &str) -> Result<PathBuf, String> {
    match args {
        [path] if !path.is_empty() => Ok(PathBuf::from(path)),
        [] => Err(format!("{command} requires exactly one path")),
        _ => Err(format!("{command} accepts exactly one path")),
    }
}

fn resolve_target_from_dir(path: &Path, cwd: &Path) -> Result<LaunchTarget, String> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    resolve_target(&path)
}

fn resolve_target(path: &Path) -> Result<LaunchTarget, String> {
    let path = canonicalize_plain(path)
        .map_err(|error| format!("could not open {}: {error}", path.display()))?;
    let metadata = path
        .metadata()
        .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
    if metadata.is_dir() {
        return Ok(LaunchTarget::Folder(path));
    }
    if !metadata.is_file() {
        return Err(format!(
            "{} is not a regular file or folder",
            path.display()
        ));
    }
    classify_supported_file_path(&path).ok_or_else(|| {
        format!(
            "{} is not a supported text, Markdown, or patch file",
            path.display()
        )
    })
}

#[derive(Clone, Copy)]
enum ExpectedKind {
    Any,
    Folder,
    File,
    Patch,
}

impl ExpectedKind {
    fn accepts(self, target: &LaunchTarget) -> bool {
        matches!(
            (self, target),
            (Self::Any, _)
                | (Self::Folder, LaunchTarget::Folder(_))
                | (
                    Self::File,
                    LaunchTarget::TextFile(_) | LaunchTarget::PatchFile(_)
                )
                | (Self::Patch, LaunchTarget::PatchFile(_))
        )
    }

    fn error_message(self) -> String {
        match self {
            Self::Any => unreachable!("Any accepts every launch target"),
            Self::Folder => "open-folder requires a folder".to_string(),
            Self::File => {
                "open-file requires a supported text, Markdown, or patch file".to_string()
            }
            Self::Patch => "open-patch requires a .patch or .diff file".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What resolution yields for an existing path: canonical, without the
    /// Windows verbatim prefix.
    fn resolved(path: &Path) -> PathBuf {
        canonicalize_plain(path).unwrap()
    }

    fn test_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "terminal-manager-launch-target-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn classifies_folder_text_markdown_and_patch_targets() {
        let root = test_root("classify");
        let folder = root.join("folder with spaces");
        let text = root.join("notes.txt");
        let markdown = root.join("README.MD");
        let patch = root.join("fix.patch");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(&text, "notes").unwrap();
        std::fs::write(&markdown, "# hello").unwrap();
        std::fs::write(&patch, "diff --git a/a b/a\n").unwrap();

        assert_eq!(
            parse_args_from_dir(
                [
                    OsString::from("open-folder"),
                    folder.clone().into_os_string()
                ],
                &root
            ),
            ParseResult::Target(LaunchTarget::Folder(resolved(&folder)))
        );
        assert_eq!(
            parse_args_from_dir(
                [OsString::from("open-file"), text.clone().into_os_string()],
                &root
            ),
            ParseResult::Target(LaunchTarget::TextFile(resolved(&text)))
        );
        assert_eq!(
            parse_args_from_dir([markdown.clone().into_os_string()], &root),
            ParseResult::Target(LaunchTarget::TextFile(resolved(&markdown)))
        );
        assert_eq!(
            parse_args_from_dir(
                [OsString::from("open-patch"), patch.clone().into_os_string()],
                &root
            ),
            ParseResult::Target(LaunchTarget::PatchFile(resolved(&patch)))
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolves_relative_paths_without_treating_other_cli_as_open_requests() {
        let root = test_root("relative");
        let text = root.join("draft.md");
        std::fs::write(&text, "draft").unwrap();

        assert_eq!(
            parse_args_from_dir(["open", "draft.md"], &root),
            ParseResult::Target(LaunchTarget::TextFile(resolved(&text)))
        );
        assert_eq!(
            parse_args_from_dir(["agent", "codex"], &root),
            ParseResult::NotRequested
        );
        for command in [
            "agent",
            "new-agent",
            "flow",
            "notify",
            "--notify",
            "activate",
            "--activate",
            "session-hook",
            "agent-notify",
            "codex-notify",
        ] {
            assert_eq!(
                parse_args_from_dir([command], &root),
                ParseResult::NotRequested,
                "{command} must reach the notification CLI parser"
            );
        }
        assert_eq!(
            parse_args_from_dir(["--bench", "pty"], &root),
            ParseResult::NotRequested
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_unsupported_or_ambiguous_requests() {
        let root = test_root("reject");
        let unsupported = root.join("data.bin");
        std::fs::write(&unsupported, [0_u8]).unwrap();

        let unsupported = parse_args_from_dir([unsupported.into_os_string()], &root);
        assert!(
            matches!(unsupported, ParseResult::Error(message) if message.contains("not a supported"))
        );
        assert!(matches!(
            parse_args_from_dir(["open-folder", "one", "two"], &root),
            ParseResult::Error(message) if message.contains("exactly one")
        ));
        assert!(matches!(
            resolve_absolute_target(Path::new("relative.txt")),
            Err(message) if message.contains("absolute")
        ));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn simplifies_verbatim_drive_and_unc_paths() {
        assert_eq!(
            simplify_verbatim_str(r"\\?\C:\Users\me\docs").as_deref(),
            Some(r"C:\Users\me\docs")
        );
        assert_eq!(simplify_verbatim_str(r"\\?\D:\").as_deref(), Some(r"D:\"));
        assert_eq!(
            simplify_verbatim_str(r"\\?\UNC\server\share\notes").as_deref(),
            Some(r"\\server\share\notes")
        );
        assert_eq!(simplify_verbatim_str(r"C:\already\plain"), None);
    }

    #[test]
    fn keeps_verbatim_paths_whose_plain_form_differs() {
        // Win32 normalization would reinterpret each of these.
        assert_eq!(simplify_verbatim_str(r"\\?\C:\dir\CON"), None);
        assert_eq!(simplify_verbatim_str(r"\\?\C:\dir\lpt1.txt"), None);
        assert_eq!(simplify_verbatim_str(r"\\?\C:\dir\trailing."), None);
        assert_eq!(simplify_verbatim_str(r"\\?\C:\dir\trailing "), None);
        assert_eq!(simplify_verbatim_str(r"\\?\C:relative"), None);
        assert_eq!(simplify_verbatim_str(r"\\?\Volume{1234}\dir"), None);
        let long = format!(r"\\?\C:\{}", "a".repeat(300));
        assert_eq!(simplify_verbatim_str(&long), None);
        // Names that merely start like a device stay simplifiable.
        assert_eq!(
            simplify_verbatim_str(r"\\?\C:\console\com10").as_deref(),
            Some(r"C:\console\com10")
        );
    }

    #[test]
    fn windows_path_keys_ignore_verbatim_prefix_case_and_separators() {
        // Regression: Explorer opens resolve to `\\?\C:\...` while saved
        // workspaces hold `C:\...`. Matching them used to need a
        // `canonicalize` of every workspace under the state lock, which hung
        // the UI on unreachable workspace paths.
        assert_eq!(
            windows_path_key(r"\\?\C:\Users\Me\Proj"),
            windows_path_key(r"c:/users/me/proj/")
        );
        assert_ne!(
            windows_path_key(r"C:\Users\me\proj"),
            windows_path_key(r"C:\Users\me\proj2")
        );
    }

    #[test]
    fn same_path_compares_without_the_filesystem() {
        let missing = Path::new("/definitely/not/a/real/workspace");
        assert!(same_path(
            missing,
            Path::new("/definitely/not/a/real/workspace/")
        ));
        assert!(!same_path(
            missing,
            Path::new("/definitely/not/a/real/other")
        ));
    }

    #[cfg(windows)]
    #[test]
    fn resolved_targets_drop_the_verbatim_prefix() {
        let root = test_root("verbatim");
        let markdown = root.join("notes.md");
        std::fs::write(&markdown, "# notes").unwrap();

        let target = resolve_absolute_target(&markdown).unwrap();
        assert!(!target.path().to_string_lossy().starts_with(r"\\?\"));
        assert!(same_path(target.path(), &markdown.canonicalize().unwrap()));

        std::fs::remove_dir_all(root).unwrap();
    }
}
