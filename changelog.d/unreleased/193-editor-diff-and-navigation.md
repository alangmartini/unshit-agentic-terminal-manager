### Added

- **Editing polish in the file editor.** `Enter` carries the line's indent
  onto the new line, keeping tabs as tabs. `Home` goes to the first
  non-blank character and to column 0 on a second press. `Tab` and
  `Shift+Tab` indent and outdent the selected lines as one undo step,
  keeping the selection so the chord repeats; with nothing selected `Tab`
  lands on the next tab stop. `Ctrl+/` comments or uncomments the selected
  lines with the language's own line comment, aligned at the shallowest
  indent of the block and leaving blank lines alone. All three are also
  palette commands (`editor.indent`, `editor.outdent`,
  `editor.toggle_comment`). Languages with no line comment — JSON, CSS,
  Markdown — have nothing for `Ctrl+/` to toggle, so there it does
  nothing; the editor sink records `editor.comment_unsupported` when it
  happens so the inert chord is tellable from a broken one.

- **Diff review pane.** `Ctrl+Shift+G` (or the palette's "Diff against…" /
  "Show uncommitted changes") opens a read-only pane showing a git range as
  a stacked unified diff, with old/new line numbers, `+`/`-` markers and
  syntax colours per file. The range accepts `HEAD`, a single revision,
  `base..head` and `base...head` (merge base); anything that is not
  plausibly a revision — anything starting with `-`, or carrying
  whitespace or shell punctuation — is refused before git runs, so a range
  can never turn into a git flag. `git diff` runs on a named worker thread
  and the pane is scrollable and closeable while it does; output is capped
  at 8 MiB and 200 000 rows. Inside the pane, `n`/`p` step hunks, `]`/`[`
  step files, and `Enter` opens the file under the cursor at its new line
  number. Lifecycle events (`diff.request`, `diff.ready`, `diff.failed`,
  `diff.nav`, `diff.open_file`) land in the profile's `diff-events.jsonl`,
  carrying the range, counts and timings — never diff content.

- **Quick open.** `Ctrl+Shift+E` opens the command palette in a new Files
  mode (`/` prefix) that fuzzy-matches every file in the workspace,
  weighting the file name above the rest of the path. The list comes from
  `git ls-files` where the workspace is a checkout and from a bounded walk
  where it is not, built on a background thread and refreshed when it is
  older than 30 seconds. Each row is the file name with its directory
  beside it, so seventeen `mod.rs` rows are still tellable apart.

- **Go to line and open-at.** `editor.goto` (palette: "Go to line…") jumps
  in the focused editor, and `editor.open_at:<line>[.<col>]:<path>` opens a
  file straight at a position — the line comes first because a Windows path
  contains a colon. Jumps centre the target when it is off screen.

- **Flow hand-offs.** A Flow Explorer node with a source location can now be
  opened in the editor (`flow.edit:<id>`) or shown inside the flow's own
  range diff, scrolled to that file and hunk (`flow.diff:<id>`). The Flow
  pane still never runs git itself; the diff pane it opens does, on a
  worker thread.

- **Find in file.** `editor.find` and friends drive a per-pane search over
  the focused document, with `editor.find_next` / `editor.find_prev` /
  `editor.find_case` stepping and configuring it. Escape closes the bar
  before any other surface.

- **Telemetry covers the new surfaces.** `quickopen.pick`,
  `editor.find_open` and `editor.find_closed` land in `editor-events.jsonl`,
  `flow.handoff` (with `kind` = `edit` or `diff`) in `flow-events.jsonl`, and
  closing a diff pane is recorded as `diff.closed` in `diff-events.jsonl`
  beside the request that opened it, instead of an `editor.close` naming a
  repository root as a file. Event names and counts only — never a
  query string, a node label, or diff content.

### Changed

- **Opening a file that is already open focuses that pane** instead of
  creating a second buffer over the same path, in any workspace, jumping to
  the requested line if one was given. Two buffers over one file meant two
  saves that could silently clobber each other.

- **Editor panes follow the theme.** Colours are re-resolved on every theme
  change instead of being frozen at the palette the pane opened with.

### Fixed

- **Read-only panes are genuinely read-only.** Typing, Enter, Tab,
  Backspace, Delete, undo/redo, paste and `Ctrl+S` all went through the
  unguarded mutation path, so a diff pane could be edited and — because its
  `path` is the repository root — a save would have tried to write the
  rendered diff over a directory. Every mutating path now goes through the
  guarded one, and `save` refuses as a last line of defence.
