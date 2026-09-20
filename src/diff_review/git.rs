//! Read-only Git range and patch queries. Called only on a worker thread.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

const MAX_OUTPUT: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Range {
    Last(usize),
    Unpushed,
    Base(String),
    Branches { from: String, to: String },
}

#[derive(Clone, Debug)]
pub struct File {
    pub path: String,
    pub old_path: Option<String>,
    pub added: Option<usize>,
    pub removed: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct Report {
    pub root: PathBuf,
    pub base: String,
    pub head: String,
    pub label: String,
    pub files: Vec<File>,
    /// Imported rows, indexed like files; absent for Git ranges.
    pub patches: Option<Vec<Vec<Line>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub old: Option<usize>,
    pub new: Option<usize>,
    pub kind: &'static str,
    pub text: String,
}

/// Bound both pipes and wall time. No shell, textconv, pager or external diff.
fn run(dir: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let mut child = crate::git::git_command(dir)
        .args([
            "--no-pager",
            "--literal-pathspecs",
            "-c",
            "core.quotePath=false",
            "-c",
            "diff.suppressBlankEmpty=false",
        ])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Cannot start Git: {e}"))?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let out_tx = tx.clone();
    let out = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout.take(MAX_OUTPUT + 1).read_to_end(&mut bytes);
        let _ = out_tx.send(bytes.len() as u64 > MAX_OUTPUT);
        result.map(|_| bytes)
    });
    let err = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stderr.take(MAX_OUTPUT + 1).read_to_end(&mut bytes);
        let _ = tx.send(bytes.len() as u64 > MAX_OUTPUT);
        result.map(|_| bytes)
    });
    let start = Instant::now();
    let status = loop {
        if rx.try_iter().any(|over| over) {
            let _ = child.kill();
            let _ = child.wait();
            break Err("Diff exceeds the 4 MiB preview limit. Select a smaller range.".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(format!("Cannot wait for Git: {e}"));
            }
            _ => {}
        }
        if start.elapsed() > Duration::from_secs(15) {
            let _ = child.kill();
            let _ = child.wait();
            break Err("Git timed out after 15 seconds. Try a smaller range.".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let output = out
        .join()
        .map_err(|_| "Git output reader failed")?
        .map_err(|e| e.to_string())?;
    let errors = err
        .join()
        .map_err(|_| "Git error reader failed")?
        .map_err(|e| e.to_string())?;
    let status = status?;
    if output.len() as u64 > MAX_OUTPUT || errors.len() as u64 > MAX_OUTPUT {
        return Err("Diff exceeds the 4 MiB preview limit. Select a smaller range.".into());
    }
    if !status.success() {
        if errors.iter().all(u8::is_ascii_whitespace) {
            return Err(if args.first() == Some(&"merge-base") {
                "These refs have no common ancestor. Choose a base ref from the same history."
                    .into()
            } else {
                format!(
                    "Git {} failed ({status}).",
                    args.first().unwrap_or(&"command")
                )
            });
        }
        return Err(String::from_utf8_lossy(&errors)
            .trim()
            .chars()
            .take(1200)
            .collect());
    }
    Ok(output)
}

fn text(dir: &Path, args: &[&str]) -> Result<String, String> {
    String::from_utf8(run(dir, args)?)
        .map(|s| s.trim_end_matches(['\r', '\n']).to_owned())
        .map_err(|_| {
            "Git returned a filename that is not UTF-8; this preview cannot display it.".into()
        })
}

fn resolve(dir: &Path, name: &str) -> Result<String, String> {
    text(
        dir,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{name}^{{commit}}"),
        ],
    )
}

fn resolve_input(dir: &Path, name: &str, field: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 1024 {
        return Err(format!(
            "Enter a branch, tag, or commit (up to 1024 bytes) as the {field}."
        ));
    }
    resolve(dir, name).map_err(|error| {
        format!("Cannot resolve {field} '{name}' to a commit. Enter a locally available branch, tag, or commit; remote branches need their remote prefix (for example, origin/trunk).\n{error}")
    })
}

/// Resolve `name`, or when empty a default from local refs only. Never fetch or guess HEAD.
fn resolve_base(dir: &Path, name: &str, field: &str) -> Result<(String, String), String> {
    let name = name.trim();
    if !name.is_empty() {
        return resolve_input(dir, name, field).map(|commit| (name.to_owned(), commit));
    }
    if let Ok(target) = text(
        dir,
        &["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"],
    ) {
        if let Ok(commit) = resolve(dir, &target) {
            return Ok((target, commit));
        }
    }
    for branch in ["refs/heads/main", "refs/heads/master"] {
        if let Ok(commit) = resolve(dir, branch) {
            return Ok((branch.into(), commit));
        }
    }
    Err("Cannot determine the default branch from local refs. Enter a base ref such as your default branch or origin/trunk.".into())
}

pub fn load(dir: &Path, range: &Range) -> Result<Report, String> {
    let root = PathBuf::from(text(dir, &["rev-parse", "--show-toplevel"])?);
    let head = match range {
        Range::Branches { to, .. } => resolve_input(&root, to, "To ref")?,
        _ => {
            resolve(&root, "HEAD").map_err(|_| "This repository has no commits yet.".to_string())?
        }
    };
    let (base, label) = match range {
        Range::Last(n) => {
            if !(1..=10_000).contains(n) {
                return Err("Choose between 1 and 10000 commits.".into());
            }
            let history = text(
                &root,
                &[
                    "rev-list",
                    "--first-parent",
                    &format!("--max-count={}", n + 1),
                    &head,
                    "--",
                ],
            )?;
            let commits: Vec<_> = history.lines().collect();
            let base = if commits.len() > *n {
                commits[*n].to_owned()
            } else if commits.len() == *n {
                let commit = text(&root, &["cat-file", "-p", commits[n - 1]])?;
                if commit
                    .split("\n\n")
                    .next()
                    .unwrap_or("")
                    .lines()
                    .any(|l| l.starts_with("parent "))
                {
                    return Err("The range crosses a shallow-history boundary. Fetch more history or select fewer commits.".into());
                }
                // hash-object reads empty stdin: works in SHA-1 and SHA-256 repos.
                text(&root, &["hash-object", "-t", "tree", "--stdin"])?
            } else {
                return Err(format!(
                    "Only {} first-parent commits are available (possibly a shallow clone).",
                    commits.len()
                ));
            };
            (base, format!("Last {n} commits · first-parent history"))
        }
        Range::Unpushed => {
            let target = text(&root, &["rev-parse", "--symbolic-full-name", "@{push}"])
                .map_err(|_| "No push target is configured for this branch. Choose a base ref, or configure the branch's push remote/tracking branch.".to_string())?;
            let target_oid = resolve(&root, &target)?;
            let base = text(&root, &["merge-base", &target_oid, &head])?;
            (
                base,
                format!("Unpushed vs {target} · locally known remote state"),
            )
        }
        Range::Base(name) => {
            let (name, target) = resolve_base(&root, name, "base ref")?;
            let base = text(&root, &["merge-base", &target, &head])?;
            (base, format!("Changes since common ancestor with {name}"))
        }
        Range::Branches { from, to } => {
            let (from, base) = resolve_base(&root, from, "From ref")?;
            (base, format!("Branch tips: {from} → {}", to.trim()))
        }
    };
    let bytes = run(
        &root,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--numstat",
            "-z",
            "--find-renames",
            &base,
            &head,
            "--",
        ],
    )?;
    let files = parse_numstat(&bytes)?;
    Ok(Report {
        patches: None,
        root,
        base,
        head,
        label,
        files,
    })
}

pub fn patch(report: &Report, file: &File) -> Result<Vec<Line>, String> {
    let mut args = vec![
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--find-renames",
        "--unified=3",
        "--output-indicator-new=+",
        "--output-indicator-old=-",
        "--output-indicator-context= ",
        &report.base,
        &report.head,
        "--",
    ];
    args.push(&file.path);
    if let Some(old) = &file.old_path {
        args.push(old);
    }
    let bytes = run(&report.root, &args)?;
    Ok(parse_patch(&String::from_utf8_lossy(&bytes)))
}

fn parse_numstat(bytes: &[u8]) -> Result<Vec<File>, String> {
    let malformed = || "Cannot parse Git's changed-file list.".to_string();
    let utf8 = |b: &[u8]| {
        String::from_utf8(b.to_vec())
            .map_err(|_| "A changed filename is not UTF-8; preview unavailable.".to_string())
    };
    let mut tokens = bytes.split(|b| *b == 0);
    let mut files = Vec::new();
    while let Some(record) = tokens.next().filter(|r| !r.is_empty()) {
        let mut parts = record.splitn(3, |b| *b == b'\t');
        let mut count = || -> Result<Option<usize>, String> {
            let value = parts.next().ok_or_else(malformed)?;
            if value == b"-" {
                Ok(None)
            } else {
                Ok(Some(utf8(value)?.parse().map_err(|_| malformed())?))
            }
        };
        let added = count()?;
        let removed = count()?;
        let name = parts.next().ok_or_else(malformed)?;
        let (old_path, path) = if name.is_empty() {
            (
                Some(utf8(tokens.next().ok_or_else(malformed)?)?),
                utf8(tokens.next().ok_or_else(malformed)?)?,
            )
        } else {
            (None, utf8(name)?)
        };
        files.push(File {
            path,
            old_path,
            added,
            removed,
        });
    }
    Ok(files)
}

pub fn parse_patch(patch: &str) -> Vec<Line> {
    let (mut old, mut new, mut in_hunk) = (0, 0, false);
    patch
        .lines()
        .map(|text| {
            let (mut old_no, mut new_no) = (None, None);
            let kind = if text.starts_with("@@ ") {
                let mut parts = text.split_whitespace().skip(1);
                let number = |s: &str| {
                    s.get(1..)
                        .and_then(|s| s.split(',').next())
                        .and_then(|n| n.parse().ok())
                };
                old = parts.next().and_then(number).unwrap_or(0);
                new = parts.next().and_then(number).unwrap_or(0);
                in_hunk = true;
                "hunk"
            } else if text.starts_with("diff --git ") {
                in_hunk = false;
                "meta"
            } else if in_hunk && text.starts_with('-') {
                old_no = Some(old);
                old += 1;
                "removed"
            } else if in_hunk && text.starts_with('+') {
                new_no = Some(new);
                new += 1;
                "added"
            } else if in_hunk && text.starts_with(' ') {
                old_no = Some(old);
                new_no = Some(new);
                old += 1;
                new += 1;
                "context"
            } else {
                "meta"
            };
            Line {
                old: old_no,
                new: new_no,
                kind,
                text: if text.chars().count() > 4096 {
                    format!(
                        "{}  [line preview truncated after 4096 characters]",
                        text.chars().take(4096).collect::<String>()
                    )
                } else {
                    text.to_owned()
                },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Repo(PathBuf);
    impl Repo {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "tm-diff-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            let repo = Self(path);
            repo.git(&["init", "-q", "-b", "main"]);
            repo.git(&["config", "user.name", "Test"]);
            repo.git(&["config", "user.email", "test@example.com"]);
            repo.git(&["config", "commit.gpgsign", "false"]);
            repo.git(&["config", "core.autocrlf", "false"]);
            repo
        }
        fn git(&self, args: &[&str]) {
            run(&self.0, args).unwrap();
        }
        fn commit(&self, contents: &str) {
            std::fs::write(self.0.join("a.txt"), contents).unwrap();
            self.git(&["add", "."]);
            self.git(&["commit", "-qm", "change"]);
        }
    }
    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn last_commits_include_root_and_ignore_dirty_files() {
        let repo = Repo::new();
        repo.commit("one\n");
        let root = load(&repo.0, &Range::Last(1)).unwrap();
        assert_eq!(root.files[0].added, Some(1));
        repo.commit("one\ntwo\n");
        std::fs::write(repo.0.join("a.txt"), "dirty\n").unwrap();
        repo.git(&["add", "."]);
        let last = load(&repo.0, &Range::Last(1)).unwrap();
        assert_eq!(last.files[0].added, Some(1));
        let lines = patch(&last, &last.files[0]).unwrap();
        assert!(lines.iter().any(|l| l.text == "+two"));
        assert!(!lines.iter().any(|l| l.text.contains("dirty")));
        assert_eq!(
            load(&repo.0, &Range::Last(2)).unwrap().files[0].added,
            Some(2)
        );
        assert!(load(&repo.0, &Range::Last(3)).is_err());
        assert!(load(&repo.0, &Range::Last(0)).is_err());
    }

    #[test]
    fn automatic_base_uses_remote_default_instead_of_assuming_main() {
        let repo = Repo::new();
        repo.commit("root\n");
        repo.git(&["branch", "-m", "trunk"]);
        repo.git(&["update-ref", "refs/remotes/origin/trunk", "HEAD"]);
        repo.git(&[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/trunk",
        ]);
        repo.git(&["checkout", "-qb", "feature"]);
        repo.commit("root\nfeature\n");
        let report = load(&repo.0, &Range::Base(String::new())).unwrap();
        assert_eq!(report.base, resolve(&repo.0, "origin/trunk").unwrap());
        assert!(report.label.contains("origin/trunk"));
        assert_eq!(report.files[0].added, Some(1));
    }

    #[test]
    fn automatic_base_falls_back_to_local_branches_and_explains_missing_default() {
        let repo = Repo::new();
        repo.commit("root\n");
        for branch in ["main", "master"] {
            repo.git(&["branch", "-m", branch]);
            let report = load(&repo.0, &Range::Base(String::new())).unwrap();
            assert!(report.files.is_empty());
            assert!(report.label.contains(branch));
        }
        repo.git(&["branch", "-m", "custom"]);
        assert!(load(&repo.0, &Range::Base(String::new()))
            .unwrap_err()
            .contains("Enter a base ref"));
    }

    #[test]
    fn invalid_base_names_the_ref_and_how_to_correct_it() {
        let repo = Repo::new();
        repo.commit("root\n");
        let error = load(&repo.0, &Range::Base("missing-branch".into())).unwrap_err();
        assert!(error.contains("missing-branch"), "{error}");
        assert!(error.contains("locally available"), "{error}");
    }

    #[test]
    fn base_comparison_excludes_changes_only_on_base_branch() {
        let repo = Repo::new();
        repo.commit("root\n");
        repo.git(&["checkout", "-qb", "feature"]);
        repo.commit("root\nfeature\n");
        repo.git(&["checkout", "-q", "main"]);
        std::fs::write(repo.0.join("base-only.txt"), "base").unwrap();
        repo.git(&["add", "."]);
        repo.git(&["commit", "-qm", "base change"]);
        repo.git(&["checkout", "-q", "feature"]);
        let report = load(&repo.0, &Range::Base("main".into())).unwrap();
        assert_eq!(report.files.len(), 1);
        assert_eq!(report.files[0].path, "a.txt");
        assert!(load(&repo.0, &Range::Base("--output=oops".into())).is_err());
        assert!(!repo.0.join("oops").exists());
        repo.git(&["checkout", "-q", "--orphan", "unrelated"]);
        repo.commit("separate history\n");
        assert!(load(&repo.0, &Range::Base("main".into()))
            .unwrap_err()
            .contains("no common ancestor"));
    }

    #[test]
    fn branch_comparison_uses_both_tips_without_checkout_or_dirty_files() {
        let repo = Repo::new();
        repo.commit("root\n");
        repo.git(&["checkout", "-qb", "feature"]);
        repo.commit("root\nfeature\n");
        repo.git(&["update-ref", "refs/remotes/origin/feature", "HEAD"]);
        repo.git(&["checkout", "-q", "main"]);
        std::fs::write(repo.0.join("base-only.txt"), "base\n").unwrap();
        repo.commit("root\n");
        std::fs::write(repo.0.join("a.txt"), "staged\n").unwrap();
        repo.git(&["add", "a.txt"]);
        std::fs::write(repo.0.join("a.txt"), "dirty\n").unwrap();
        let before = run(&repo.0, &["status", "--porcelain=v1", "-z"]).unwrap();
        let range = Range::Branches {
            from: " main ".into(),
            to: "origin/feature".into(),
        };
        let report = load(&repo.0, &range).unwrap();
        assert_eq!(report.base, resolve(&repo.0, "main").unwrap());
        assert_eq!(report.head, resolve(&repo.0, "feature").unwrap());
        assert!(report.label.contains("main → origin/feature"));
        assert_eq!(report.files.len(), 2);
        let removed = report
            .files
            .iter()
            .find(|f| f.path == "base-only.txt")
            .unwrap();
        assert_eq!(removed.removed, Some(1));
        let file = report.files.iter().find(|f| f.path == "a.txt").unwrap();
        let lines = patch(&report, file).unwrap();
        assert!(lines.iter().any(|l| l.text == "+feature"));
        assert!(!lines
            .iter()
            .any(|l| l.text.contains("dirty") || l.text.contains("staged")));
        let reverse = load(
            &repo.0,
            &Range::Branches {
                from: "feature".into(),
                to: "main".into(),
            },
        )
        .unwrap();
        assert_eq!(reverse.base, report.head);
        assert_eq!(reverse.head, report.base);
        assert_eq!(
            run(&repo.0, &["status", "--porcelain=v1", "-z"]).unwrap(),
            before
        );
        assert_eq!(
            text(&repo.0, &["symbolic-ref", "--short", "HEAD"]).unwrap(),
            "main"
        );
    }

    #[test]
    fn branch_comparison_handles_defaults_unrelated_history_and_invalid_refs() {
        let repo = Repo::new();
        repo.commit("root\n");
        let report = load(
            &repo.0,
            &Range::Branches {
                from: String::new(),
                to: "HEAD".into(),
            },
        )
        .unwrap();
        assert!(report.files.is_empty());
        repo.git(&["checkout", "-q", "--orphan", "unrelated"]);
        repo.commit("separate history\n");
        let report = load(
            &repo.0,
            &Range::Branches {
                from: "main".into(),
                to: "unrelated".into(),
            },
        )
        .unwrap();
        assert_eq!(report.files.len(), 1);
        for (from, to, field) in [
            ("missing", "HEAD", "From ref"),
            ("main", "missing", "To ref"),
            ("main", "", "To ref"),
            ("main", "--output=oops", "To ref"),
            ("main", "main..unrelated", "To ref"),
        ] {
            let error = load(
                &repo.0,
                &Range::Branches {
                    from: from.into(),
                    to: to.into(),
                },
            )
            .unwrap_err();
            assert!(error.contains(field), "{error}");
        }
        assert!(!repo.0.join("oops").exists());
    }

    #[test]
    fn unpushed_uses_remote_tracking_ref_and_reports_missing_target() {
        let repo = Repo::new();
        repo.commit("root\n");
        assert!(load(&repo.0, &Range::Unpushed)
            .unwrap_err()
            .contains("push target"));
        repo.git(&[
            "remote",
            "add",
            "origin",
            "https://example.invalid/repo.git",
        ]);
        repo.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        repo.git(&["config", "branch.main.remote", "origin"]);
        repo.git(&["config", "branch.main.merge", "refs/heads/main"]);
        repo.git(&["config", "push.default", "simple"]);
        assert!(load(&repo.0, &Range::Unpushed).unwrap().files.is_empty());
        repo.commit("root\nlocal\n");
        assert_eq!(
            load(&repo.0, &Range::Unpushed).unwrap().files[0].added,
            Some(1)
        );
    }

    #[test]
    fn renamed_and_binary_files_load_with_literal_paths() {
        let repo = Repo::new();
        repo.commit("root\n");
        repo.git(&["mv", "a.txt", "[literal].txt"]);
        std::fs::write(repo.0.join("binary"), b"\0\x01\x02").unwrap();
        repo.git(&["add", "."]);
        repo.git(&["commit", "-qm", "rename and binary"]);
        let report = load(&repo.0, &Range::Last(1)).unwrap();
        let renamed = report.files.iter().find(|f| f.old_path.is_some()).unwrap();
        assert_eq!(renamed.path, "[literal].txt");
        assert!(patch(&report, renamed)
            .unwrap()
            .iter()
            .any(|l| l.text.starts_with("rename to")));
        let binary = report.files.iter().find(|f| f.path == "binary").unwrap();
        assert_eq!(binary.added, None);
        assert!(patch(&report, binary)
            .unwrap()
            .iter()
            .any(|l| l.text.contains("Binary files")));
    }

    #[test]
    fn numstat_preserves_tabs_newlines_and_rename_paths() {
        let files =
            parse_numstat(b"2\t1\todd\tname\n.rs\0-\t-\timage.png\x000\t0\t\0old name\0new name\0")
                .unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].path, "odd\tname\n.rs");
        assert_eq!(files[0].added, Some(2));
        assert_eq!(files[1].added, None);
        assert_eq!(files[2].old_path.as_deref(), Some("old name"));
        assert_eq!(files[2].path, "new name");
    }

    #[test]
    fn patch_numbers_do_not_treat_content_as_headers() {
        let lines = parse_patch("--- a/a\n+++ b/a\n@@ -4,2 +4,2 @@ fn x\n--- text\n+++ text\n same\n\\ No newline at end of file\n");
        assert_eq!(lines[3].old, Some(4));
        assert_eq!(lines[3].new, None);
        assert_eq!(lines[4].new, Some(4));
        assert_eq!(lines[5].old, Some(5));
        assert_eq!(lines[5].new, Some(5));
        assert_eq!(lines[6].old, None);
    }
}
