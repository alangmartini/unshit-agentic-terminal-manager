//! Lightweight git branch detection used by the sidebar to decorate
//! terminals whose cwd lives inside a repository.
//!
//! Every `git` invocation in this crate must go through [`git_command`].
//! The release build is a `windows_subsystem = "windows"` binary, so it owns
//! no console: each child process spawned without `CREATE_NO_WINDOW` makes
//! Windows allocate a **new console window**, which flashes on screen for the
//! lifetime of the child. Seven restored workspaces meant seven black windows
//! blinking before the UI appeared. [`assert_all_git_spawns_are_silent`]
//! enforces the rule so a future call site cannot bring the flashing back.

use std::path::Path;
use std::process::Command;

/// A `git` command that never allocates a console window.
///
/// Prefer this over `Command::new("git")` everywhere, including tests -- the
/// source guard below rejects the bare form.
pub fn git_command(dir: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(dir);
    apply_no_window(&mut cmd);
    cmd
}

/// Suppress console allocation for a child process.
///
/// `CREATE_NO_WINDOW` is the documented flag for "run this console
/// application without giving it a console". Mirrors the daemon spawn path in
/// [`crate::daemon`].
#[cfg(windows)]
pub fn apply_no_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
pub fn apply_no_window(_cmd: &mut Command) {}

/// Return the current branch name for the git repository containing `path`,
/// or `None` if `path` is not a directory, is not inside a repo, git is not
/// installed, or HEAD is detached.
///
/// Spawning a process costs ~30ms on Windows, so this is deliberately never
/// called on the startup path; see `crate::git_watch`.
pub fn detect_git_branch(path: &Path) -> Option<String> {
    if !path.is_dir() {
        return None;
    }

    let output = git_command(path)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // `git rev-parse --abbrev-ref HEAD` prints the literal string `HEAD`
    // when the working copy is in a detached-HEAD state; treat that as
    // "no branch" so callers can render it the same as a non-repo cwd.
    if branch.is_empty() || branch == "HEAD" {
        return None;
    }

    Some(branch)
}

/// Fail if any source file spawns `git` without going through
/// [`git_command`]. Returns the offending `path:line` locations.
///
/// This is the only practical way to test the invariant: `std::process::
/// Command` exposes no getter for creation flags, so an assertion on a built
/// command is impossible, and the symptom (a console window flashing) is not
/// observable from a test process that already owns a console.
#[cfg(test)]
fn assert_all_git_spawns_are_silent() -> Vec<String> {
    fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    walk(&src, &mut files);

    let mut offenders = Vec::new();
    for file in files {
        // This module defines the sanctioned wrapper; it is the one place the
        // bare form is allowed to appear.
        if file.file_name().is_some_and(|n| n == "git.rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            if line.contains(r#"Command::new("git")"#) {
                let rel = file.strip_prefix(&src).unwrap_or(&file);
                offenders.push(format!("{}:{}", rel.display(), i + 1));
            }
        }
    }
    offenders
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique_temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut dir = std::env::temp_dir();
        dir.push(format!("terminal-manager-git-{tag}-{nanos}-{seq}"));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn run_git(dir: &Path, args: &[&str]) {
        let status = git_command(dir).args(args).status().expect("run git");
        assert!(status.success(), "git {:?} failed in {:?}", args, dir);
    }

    fn init_repo(dir: &Path) {
        run_git(dir, &["init", "-q"]);
        run_git(dir, &["config", "user.email", "test@example.com"]);
        run_git(dir, &["config", "user.name", "Test"]);
        run_git(dir, &["config", "commit.gpgsign", "false"]);
        run_git(dir, &["commit", "--allow-empty", "-q", "-m", "x"]);
    }

    #[test]
    fn returns_branch_for_initialized_repo() {
        let dir = unique_temp_dir("init");
        init_repo(&dir);

        let branch = detect_git_branch(&dir).expect("branch detected");
        assert!(
            branch == "main" || branch == "master",
            "unexpected default branch: {branch:?}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn returns_none_for_non_repo_directory() {
        let dir = unique_temp_dir("plain");
        assert!(detect_git_branch(&dir).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn returns_none_for_missing_path() {
        let missing = PathBuf::from("/definitely/does/not/exist/terminal-manager-git");
        assert!(detect_git_branch(&missing).is_none());
    }

    #[test]
    fn returns_none_for_detached_head() {
        let dir = unique_temp_dir("detached");
        init_repo(&dir);
        // Detach HEAD onto the commit we just made so `rev-parse
        // --abbrev-ref HEAD` prints the literal string "HEAD".
        run_git(&dir, &["checkout", "-q", "--detach", "HEAD"]);

        assert!(detect_git_branch(&dir).is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    /// The release binary owns no console, so a `git` child spawned without
    /// `CREATE_NO_WINDOW` pops a real window on the user's screen. Catch a new
    /// call site here rather than in a bug report about "terminals flashing".
    #[test]
    fn every_git_spawn_goes_through_the_silent_helper() {
        let offenders = assert_all_git_spawns_are_silent();
        assert!(
            offenders.is_empty(),
            "these sites spawn git without CREATE_NO_WINDOW and will flash a \
             console window in the release build; use crate::git::git_command \
             instead:\n  {}",
            offenders.join("\n  ")
        );
    }
}
