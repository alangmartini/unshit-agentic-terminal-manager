//! Lightweight git branch detection used by the sidebar to decorate
//! terminals whose cwd lives inside a repository.

use std::path::Path;
use std::process::Command;

/// Return the current branch name for the git repository containing `path`,
/// or `None` if `path` is not a directory, is not inside a repo, git is not
/// installed, or HEAD is detached.
pub fn detect_git_branch(path: &Path) -> Option<String> {
    if !path.is_dir() {
        return None;
    }

    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(path)
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
}
