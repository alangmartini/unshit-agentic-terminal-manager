# Local Git diff review

Provide a GitHub-style review overlay opened from the titlebar or command palette.
Use the focused session's recorded launch directory (including worktree tabs),
falling back to the active workspace directory. Capture it when opening.

## Behavior
- Open a UTF-8 Git-format `.patch`/`.diff` file using **Open patch...**, or drop
  one file onto the open review. IntelliJ and format-patch preambles are ignored;
  paths, hunks, renames, additions/deletions and binary notices reuse the shared
  diff parser. No repository or target files are required and nothing is applied.
- Imported files use the same filter, unified/split views, hunk navigation and
  viewed marks. Show the patch path instead of a repository/range. Refresh rereads
  the patch; selecting a Git mode returns to the captured repository. Imports run
  on the existing worker with stale-result rejection and a 4 MiB file limit.
  Unsupported/empty formats, invalid UTF-8 and oversized files show errors.
  Long lines use the shared parser's visible clipping notice; row-limit overflow
  is rejected. Counts refer to displayed changes.
- Previous/Next hunk controls navigate within the open file, highlight the target,
  and show its ordinal/total. A jump starts a bounded row window at the hunk header
  in either view, without another Git query. Switching views keeps a selected hunk.
  Ordinary row paging clears hunk selection; File start returns to the beginning.
  Loading another file/range resets navigation; binary/metadata-only patches have
  no navigable hunks. Row-range labels make non-page-aligned jumps explicit.
- Filter the changed-file list by case-insensitive substring of the current or
  former path (rename), accepting either slash style. Show matching/total counts,
  clear control, and no-results text. Filtering keeps the open patch and its page,
  marks when it is outside the filter, resets file-list pagination, and never runs
  Git. Cache matching original file indices on filter/report changes.
- Unified / Side by side toggle keeps the loaded range and file. Side by side
  pairs each contiguous deletion/addition block by position, preserves hunk and
  binary metadata, and attaches no-final-newline notes to the correct side.
  Align before pagination; share vertical scrolling and use a minimum column
  width with horizontal scrolling on narrow windows. Default remains unified.
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
Files can be marked viewed and unmarked from their patch header. Show viewed
labels in the sidebar and a whole-range progress count independent of filtering.
Keep marks across file navigation and layout changes; clear them on range refresh
or close. Disable marking while loading or after a patch error. No persistence
or Git writes are involved.

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

Set `TM_DIFF_VISUAL_PATCH` with `TM_DIFF_VISUAL_DUMP` to capture an imported patch
through the existing GPU review screenshot test.
