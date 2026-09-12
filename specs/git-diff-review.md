# Local Git diff review

Provide a GitHub-style review overlay opened from the titlebar or command palette.
Use the focused session's recorded launch directory (including worktree tabs),
falling back to the active workspace directory. Capture it when opening.

## Behavior
- Last N commits (first-parent history), unpushed commits against the locally
  known push target, and a base branch/ref comparison using merge-base.
- Show the exact resolved range, file list, additions/deletions, and a unified
  patch with old/new line numbers. Load only the selected file's patch.
- Preserve rename paths, binary notices, empty states and actionable Git errors.
- Refresh explicitly. Never fetch, push, stage, or alter the working directory.
- Run Git off the UI thread; discard stale responses after selection or close.
- Bound process runtime/output and paginate patch rows to bound UI tree size.
- Escape closes review; typing into range fields never reaches a terminal.

## Implementation and verification
Rust and the existing native unshit element/CSS toolkit; no new dependencies.
`src/diff_review/` owns Git queries and review state; `src/ui/diff_review.rs`
renders it. App snapshot, titlebar and palette provide integration.
Follow existing `git_command(dir).args([...])` and `mutate_with` conventions.

Implement/test Git ranges and patch parsing first, then async state and UI.
Use temporary Git repositories for root commits, merges, push targets, renames,
binary files, odd filenames, invalid ranges, and dirty-worktree exclusion.
Use the framework test harness for layout and interaction; launch a separate
app profile for a screenshot when practical.

Commands: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`,
and `cargo run` under an isolated `TM_PROFILE`.

Always preserve PTY lifecycle and render invariants. No changes to dependencies,
daemon ownership, persistence or framework behavior are needed. Publication and
remote Git operations are outside this feature's scope.
