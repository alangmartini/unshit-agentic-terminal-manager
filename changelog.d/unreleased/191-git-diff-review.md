### Added

- **Git diff review.** "Review Git diff" in the titlebar and the command
  palette opens a review overlay over a range of committed work: the last
  N commits along first-parent history, the commits ahead of the locally
  known push target, or everything since the merge base with a base
  branch or ref. The overlay lists the changed files with their
  additions and deletions and shows one file's patch at a time, with old
  and new line numbers, rename paths, binary-file notices and the exact
  resolved range spelled out. Unified and side-by-side views both keep
  the loaded range, file and selected hunk when you switch between them;
  side by side pairs contiguous deletion and addition blocks by position
  and shares vertical scrolling. `Previous`/`Next hunk` move within the
  open file and say which hunk of how many you are on, filtering narrows
  the file list by a case-insensitive substring of either the current or
  the pre-rename path, and files can be marked viewed — with undo — so
  progress across the range is visible. Long patches page rather than
  render whole.

  Review is read-only: it never fetches, pushes, stages or touches the
  working directory, and it excludes staged and working-tree edits by
  design. Git runs on a background worker with request coalescing, stale
  responses discarded after a selection or close, and runtime and output
  limits, so the overlay stays closeable while a large range resolves.
  The range is taken from the focused session's recorded launch
  directory — worktree tabs included — captured when the overlay opens.

  Commands live under the `review.*` prefix (`review.open`,
  `review.viewed`, `review.hunk_next` and the rest); the editor's own
  diff pane keeps `diff.*`.
