//! Running `git diff` for a diff pane, off the UI thread.
//!
//! Every other git call in this crate is synchronous because it is a
//! single `rev-parse` on a directory. A diff of a large range is not: it
//! can take hundreds of milliseconds and produce megabytes, so it runs on
//! a worker thread (`crate::state` owns the spawn, mirroring
//! `crate::git_watch`) and this module stays free of app state so it can
//! be tested directly.

use std::io::Read;
use std::path::Path;
use std::process::Stdio;
use std::time::Instant;

use super::parse::{parse_unified_diff, DiffDocument};
use super::spec::DiffSpec;

/// Stop reading git's stdout past this. A diff bigger than this is not
/// reviewable in a pane anyway, and the bytes would sit in memory twice
/// (raw plus parsed document).
pub const MAX_DIFF_STDOUT_BYTES: usize = 8 * 1024 * 1024;

/// What a finished diff job produced.
#[derive(Debug)]
pub enum DiffOutcome {
    Ready {
        document: DiffDocument,
        stdout_bytes: usize,
        /// Git produced more output than [`MAX_DIFF_STDOUT_BYTES`] and
        /// the tail was dropped.
        stdout_truncated: bool,
        elapsed_ms: u64,
    },
    Failed {
        /// Machine-readable, for telemetry queries: `git_error`, `spawn`,
        /// `too_large`.
        reason: &'static str,
        /// User-facing message; git's own first line of stderr when it
        /// has one, because "unknown revision 'mian'" is exactly what the
        /// user needs to read.
        message: String,
        elapsed_ms: u64,
    },
}

/// Run `git diff` for `spec` in `repo_root` and parse the result.
///
/// Blocks; call from a worker thread.
pub fn run_diff(spec: &DiffSpec, repo_root: &Path) -> DiffOutcome {
    let started = Instant::now();
    let elapsed = |start: Instant| start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;

    let mut cmd = crate::git::git_command(repo_root);
    // `-c core.quotepath=false` keeps non-ASCII paths readable instead of
    // octal-escaped. `--no-ext-diff` ignores a user's configured external
    // differ, which would produce something this parser cannot read (and
    // could launch a GUI). `--no-color` because a config may force colour
    // even when stdout is not a tty.
    cmd.args([
        "-c",
        "core.quotepath=false",
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--find-renames",
        "-U3",
    ]);
    for arg in spec.git_args() {
        cmd.arg(arg);
    }
    // Ends the revision list: nothing after this can be read as a
    // pathspec, and nothing in `git_args` can be read as a flag
    // (`DiffSpec` rejects a leading `-`).
    cmd.arg("--");

    let mut child = match cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(child) => child,
        Err(e) => {
            return DiffOutcome::Failed {
                reason: "spawn",
                message: format!("could not run git: {e}"),
                elapsed_ms: elapsed(started),
            }
        }
    };

    // Read at most the cap plus one byte, so "exactly the cap" is not
    // reported as truncated.
    let mut stdout = Vec::new();
    if let Some(pipe) = child.stdout.take() {
        if let Err(e) = pipe
            .take(MAX_DIFF_STDOUT_BYTES as u64 + 1)
            .read_to_end(&mut stdout)
        {
            return DiffOutcome::Failed {
                reason: "git_error",
                message: format!("could not read git output: {e}"),
                elapsed_ms: elapsed(started),
            };
        }
    }
    let stdout_truncated = stdout.len() > MAX_DIFF_STDOUT_BYTES;
    if stdout_truncated {
        stdout.truncate(MAX_DIFF_STDOUT_BYTES);
        // Dropping our end of the pipe makes git stop writing; it exits
        // with a write error, which is expected here and not a failure.
        let _ = child.kill();
    }

    // `wait_with_output` drains stderr and reaps the child. stdout was
    // taken above, so this only reads the error stream.
    let finished = match child.wait_with_output() {
        Ok(finished) => finished,
        Err(e) => {
            return DiffOutcome::Failed {
                reason: "git_error",
                message: format!("git did not finish: {e}"),
                elapsed_ms: elapsed(started),
            }
        }
    };

    if !finished.status.success() && !stdout_truncated {
        let stderr = String::from_utf8_lossy(&finished.stderr);
        let message = stderr
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("git diff failed")
            .to_string();
        return DiffOutcome::Failed {
            reason: "git_error",
            message: strip_git_prefix(&message),
            elapsed_ms: elapsed(started),
        };
    }

    // Git output is UTF-8 for text files; a diff of a latin-1 file can
    // carry invalid sequences. Lossy conversion keeps the rest readable
    // rather than failing the whole review.
    let stdout_bytes = stdout.len();
    let text = String::from_utf8_lossy(&stdout);
    let document = parse_unified_diff(&text);
    DiffOutcome::Ready {
        document,
        stdout_bytes,
        stdout_truncated,
        elapsed_ms: elapsed(started),
    }
}

/// Trim git's `fatal: ` / `error: ` prefix — the app surfaces the message
/// in a toast that already reads as a failure.
fn strip_git_prefix(message: &str) -> String {
    for prefix in ["fatal: ", "error: ", "warning: "] {
        if let Some(rest) = message.strip_prefix(prefix) {
            return rest.to_string();
        }
    }
    message.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_repo(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tm-diff-job-{}-{}-{}",
            tag,
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create repo dir");
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "test@example.com"],
            vec!["config", "user.name", "Test"],
            vec!["config", "commit.gpgsign", "false"],
        ] {
            let status = crate::git::git_command(&dir)
                .args(&args)
                .status()
                .expect("run git");
            assert!(status.success(), "git {args:?} failed");
        }
        dir
    }

    fn commit(dir: &Path, message: &str) {
        for args in [vec!["add", "-A"], vec!["commit", "-q", "-m", message]] {
            let status = crate::git::git_command(dir)
                .args(&args)
                .status()
                .expect("run git");
            assert!(status.success(), "git {args:?} failed");
        }
    }

    /// The default range: uncommitted work against HEAD. This is what
    /// `diff.open` with no argument shows, so it has to work on a repo
    /// with edits that were never staged.
    #[test]
    fn working_tree_diff_reports_an_uncommitted_edit() {
        let dir = temp_repo("worktree");
        fs::write(dir.join("a.rs"), "fn main() {}\n").expect("write");
        commit(&dir, "initial");
        fs::write(dir.join("a.rs"), "fn main() {\n    let x = 1;\n}\n").expect("edit");

        let spec = DiffSpec::parse("HEAD").expect("spec");
        match run_diff(&spec, &dir) {
            DiffOutcome::Ready { document, .. } => {
                assert_eq!(document.files.len(), 1, "one changed file");
                assert!(document.files[0].path.ends_with("a.rs"));
                assert!(
                    document
                        .rows
                        .iter()
                        .any(|r| matches!(r.kind, crate::diff::DiffRowKind::Added)),
                    "the added line must show up"
                );
            }
            other => panic!("expected a diff, got {other:?}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// A clean tree is not an error — the pane says "no changes".
    #[test]
    fn a_clean_tree_yields_an_empty_document() {
        let dir = temp_repo("clean");
        fs::write(dir.join("a.txt"), "same\n").expect("write");
        commit(&dir, "initial");

        let spec = DiffSpec::parse("HEAD").expect("spec");
        match run_diff(&spec, &dir) {
            DiffOutcome::Ready { document, .. } => {
                assert!(document.files.is_empty());
                assert!(document.lines.is_empty());
            }
            other => panic!("expected an empty diff, got {other:?}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// A two-commit range must diff the commits, not the working tree.
    #[test]
    fn a_committed_range_diffs_the_two_revisions() {
        let dir = temp_repo("range");
        fs::write(dir.join("a.txt"), "one\n").expect("write");
        commit(&dir, "first");
        fs::write(dir.join("a.txt"), "two\n").expect("write");
        commit(&dir, "second");

        let spec = DiffSpec::parse("HEAD~1..HEAD").expect("spec");
        match run_diff(&spec, &dir) {
            DiffOutcome::Ready { document, .. } => {
                assert_eq!(document.files.len(), 1);
                let text = document.lines.join("\n");
                assert!(text.contains("one") && text.contains("two"), "{text}");
            }
            other => panic!("expected a diff, got {other:?}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// A bad revision has to surface git's own message: the user needs to
    /// see "unknown revision", not a generic failure.
    #[test]
    fn an_unknown_revision_fails_with_gits_message() {
        let dir = temp_repo("badrev");
        fs::write(dir.join("a.txt"), "x\n").expect("write");
        commit(&dir, "initial");

        let spec = DiffSpec::parse("no-such-ref-anywhere").expect("spec");
        match run_diff(&spec, &dir) {
            DiffOutcome::Failed {
                reason, message, ..
            } => {
                assert_eq!(reason, "git_error");
                assert!(!message.is_empty(), "git's message must be surfaced");
                assert!(
                    !message.starts_with("fatal:"),
                    "the fatal: prefix is stripped, got {message:?}"
                );
            }
            other => panic!("expected a failure, got {other:?}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn running_outside_a_repository_fails_rather_than_hanging() {
        let dir = std::env::temp_dir().join(format!(
            "tm-diff-job-norepo-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create dir");

        let spec = DiffSpec::parse("HEAD").expect("spec");
        // A temp dir can itself sit inside a checkout on some machines;
        // only assert the failure shape when git agrees it is not a repo.
        if crate::git::repo_root(&dir).is_none() {
            match run_diff(&spec, &dir) {
                DiffOutcome::Failed { reason, .. } => assert_eq!(reason, "git_error"),
                other => panic!("expected a failure outside a repo, got {other:?}"),
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn git_message_prefixes_are_stripped() {
        assert_eq!(strip_git_prefix("fatal: bad revision"), "bad revision");
        assert_eq!(strip_git_prefix("error: nope"), "nope");
        assert_eq!(strip_git_prefix("plain"), "plain");
    }
}
