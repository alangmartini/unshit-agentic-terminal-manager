//! Worktree preparation and agent spawn helpers for Quick Prompt.
//!
//! Submit-time flow:
//! 1. `prepare_target_in(base, workspace_cwd)` decides whether the
//!    active workspace is inside a git repo. If yes it shells out
//!    `git worktree add <path> HEAD` so the agent runs on a fresh
//!    anonymous branch without disturbing the user's checkout. If no
//!    (or no workspace cwd at all), it creates a plain directory at
//!    the same path so the agent still has an empty cwd to work in.
//! 2. `claude_shell_spec(prompt)` builds the `ShellSpec` the daemon
//!    will exec when spawning the new tab. Codex parity lands in
//!    Slice 6.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::shell::ShellSpec;

/// Resolved target directory for the agent to run in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetDir {
    pub path: PathBuf,
    pub kind: TargetKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind {
    /// Created via `git worktree add` so the agent has a real branch.
    Worktree,
    /// Plain directory; the workspace was not a git repo or had no
    /// path at all (empty repo fallback per spec A5.3).
    PlainDir,
}

#[cfg(windows)]
const CLAUDE_PROGRAM: &str = "claude.cmd";
#[cfg(not(windows))]
const CLAUDE_PROGRAM: &str = "claude";

/// Production base for Quick Prompt worktrees:
/// `%APPDATA%\com.godly.terminal\worktrees` on Windows; matching
/// platform-specific data dir elsewhere.
pub fn worktree_base() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("com.godly.terminal").join("worktrees"))
}

/// Generate a fresh `godly-qp-<8-hex>` directory name. The hex draws
/// from the system clock + PID + a process-local counter so collisions
/// between two submits in the same millisecond are still avoided.
fn generate_target_name() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u128;
    let mixed = (nanos.wrapping_mul(0x9E3779B97F4A7C15))
        .wrapping_add(pid.wrapping_mul(0x100000001B3))
        .wrapping_add(n as u128);
    format!("godly-qp-{:08x}", mixed as u32)
}

/// Convenience wrapper over `prepare_target_in` that uses the
/// production `%APPDATA%` base path.
pub fn prepare_target(workspace_cwd: Option<&Path>) -> io::Result<TargetDir> {
    let base = worktree_base().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "could not determine APPDATA worktree base",
        )
    })?;
    prepare_target_in(&base, workspace_cwd)
}

/// Prepare a worktree (or plain dir) under `base`. Tests pass a temp
/// directory so they do not pollute the user's APPDATA.
pub fn prepare_target_in(base: &Path, workspace_cwd: Option<&Path>) -> io::Result<TargetDir> {
    let path = base.join(generate_target_name());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let in_repo = workspace_cwd
        .filter(|p| p.exists())
        .is_some_and(is_inside_work_tree);

    if in_repo {
        let cwd = workspace_cwd.expect("filtered above");
        let path_str = path
            .to_str()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "non-utf8 worktree path"))?;
        let output = Command::new("git")
            .args(["worktree", "add", path_str, "HEAD"])
            .current_dir(cwd)
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "git worktree add failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(TargetDir {
            path,
            kind: TargetKind::Worktree,
        })
    } else {
        std::fs::create_dir_all(&path)?;
        Ok(TargetDir {
            path,
            kind: TargetKind::PlainDir,
        })
    }
}

/// `git rev-parse --is-inside-work-tree` against `path`. Returns false
/// for non-existent paths, non-repo dirs, and any error from git.
fn is_inside_work_tree(path: &Path) -> bool {
    let Ok(output) = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(path)
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    String::from_utf8_lossy(&output.stdout).trim() == "true"
}

/// Build the `ShellSpec` for `claude <prompt>`. On Windows the program
/// is `claude.cmd` so PathExt resolution still finds it when the user
/// installed Claude through the standard installer. On other platforms
/// it is just `claude`.
pub fn claude_shell_spec(prompt: &str) -> ShellSpec {
    ShellSpec {
        program: CLAUDE_PROGRAM.to_string(),
        args: vec![prompt.to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_temp_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("godly-qp-spawn-{tag}-{pid}-{n}-{nanos}"));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn run_git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .expect("run git");
        assert!(status.success(), "git {:?} failed in {:?}", args, dir);
    }

    fn init_repo(dir: &Path) {
        run_git(dir, &["init", "-q"]);
        run_git(dir, &["config", "user.email", "test@example.com"]);
        run_git(dir, &["config", "user.name", "Test"]);
        run_git(dir, &["config", "commit.gpgsign", "false"]);
        run_git(dir, &["commit", "--allow-empty", "-q", "-m", "x"]);
    }

    // --- Pure helper tests ----------------------------------------------

    #[test]
    fn generate_target_name_is_godly_qp_prefixed() {
        let name = generate_target_name();
        assert!(name.starts_with("godly-qp-"), "got: {}", name);
        assert_eq!(name.len(), "godly-qp-".len() + 8);
        assert!(name[9..].chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn generate_target_name_collisions_are_unlikely() {
        // Sequential calls in the same nanosecond use the counter; we
        // expect two consecutive calls to differ.
        let a = generate_target_name();
        let b = generate_target_name();
        assert_ne!(a, b);
    }

    #[test]
    fn claude_shell_spec_uses_prompt_as_arg() {
        let spec = claude_shell_spec("say hi");
        assert_eq!(spec.program, CLAUDE_PROGRAM);
        assert_eq!(spec.args, vec!["say hi".to_string()]);
    }

    #[test]
    fn claude_shell_spec_preserves_multiline_prompts() {
        let spec = claude_shell_spec("line one\nline two");
        assert_eq!(spec.args, vec!["line one\nline two".to_string()]);
    }

    // --- prepare_target_in ----------------------------------------------

    #[test]
    fn prepare_target_in_creates_plain_dir_when_workspace_cwd_is_none() {
        let base = unique_temp_dir("plain-none");
        let result = prepare_target_in(&base, None).expect("prepare");
        assert_eq!(result.kind, TargetKind::PlainDir);
        assert!(result.path.exists(), "path should be created on disk");
        assert!(result.path.is_dir());
        assert!(result.path.starts_with(&base));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn prepare_target_in_creates_plain_dir_when_workspace_is_not_a_repo() {
        let base = unique_temp_dir("plain-non-repo");
        let workspace = unique_temp_dir("plain-non-repo-ws");
        let result = prepare_target_in(&base, Some(&workspace)).expect("prepare");
        assert_eq!(result.kind, TargetKind::PlainDir);
        assert!(result.path.exists());
        assert!(result.path.is_dir());
        let _ = std::fs::remove_dir_all(&base);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn prepare_target_in_creates_plain_dir_when_workspace_path_missing() {
        let base = unique_temp_dir("plain-missing");
        let workspace = unique_temp_dir("plain-missing-ws");
        std::fs::remove_dir_all(&workspace).ok();
        let result = prepare_target_in(&base, Some(&workspace)).expect("prepare");
        assert_eq!(result.kind, TargetKind::PlainDir);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn prepare_target_in_creates_worktree_when_workspace_is_a_repo() {
        let base = unique_temp_dir("worktree-base");
        let workspace = unique_temp_dir("worktree-repo");
        init_repo(&workspace);

        let result = prepare_target_in(&base, Some(&workspace)).expect("prepare");
        assert_eq!(result.kind, TargetKind::Worktree);
        assert!(result.path.exists(), "worktree path should exist");
        // git worktree leaves a `.git` file (not dir) pointing at the
        // primary repo's worktree metadata.
        let git_marker = result.path.join(".git");
        assert!(
            git_marker.exists(),
            ".git marker should exist inside the worktree"
        );

        // Clean up: remove the worktree from git's bookkeeping before
        // dropping the temp dirs so we do not leave the source repo
        // referencing a missing worktree.
        let _ = Command::new("git")
            .args([
                "worktree",
                "remove",
                "--force",
                result.path.to_str().unwrap(),
            ])
            .current_dir(&workspace)
            .status();
        let _ = std::fs::remove_dir_all(&base);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn is_inside_work_tree_returns_false_for_plain_dir() {
        let dir = unique_temp_dir("plain");
        assert!(!is_inside_work_tree(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_inside_work_tree_returns_true_for_repo() {
        let dir = unique_temp_dir("repo");
        init_repo(&dir);
        assert!(is_inside_work_tree(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
