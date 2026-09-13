# Editor v2: highlighting, diff review pane and navigation

Status: implementing (2026-09-09). Builds on the editor MVP in
[`SPEC.md`](../SPEC.md) (shipped in v0.4.0) and on the Flow Explorer
([`specs/flow-explorer.md`](flow-explorer.md)).

## Objective

Make the in-app editor good enough to *review a change* and to *edit a
simple document* without leaving the terminal manager, in the spirit of
Zed: instant, keyboard-first, no language servers.

The request behind this spec, verbatim: "finally implement a file editor
capability here, with some small navigation … similar to how Zed operates
(super fast and smart) … mainly to see PRs diffs and etc when using flow
feature, but also to be able to edit simple documents. No need for complex
language servers setup and etc."

Interpretation (the user was not available for questions):

1. **Primary:** a diff viewer that the Flow review mode can open for its
   `diff_range`, and that stands on its own for "what did I change".
2. **Secondary:** small navigation: open a file fast, go to a line, find in
   the file, jump between hunks/files, open a Flow node's location.
3. **Baseline:** the existing editor gets syntax colours, theme-following
   colours, and the clipboard chords that are broken today.

Explicitly out of scope (unchanged from the MVP deferrals): LSP or any
language intelligence, soft wrap, multi-cursor, side-by-side or editable
diffs, a file-tree sidebar, editor/diff pane persistence across restarts,
editor panes inside terminal splits, wide-glyph cell mapping, IME preedit,
new crate dependencies.

## Surfaces

### 1. Clipboard chords in editor panes (bug fix)

Today `Ctrl+V`, `Ctrl+Shift+V` and `Shift+Insert` are system bindings for
`terminal.paste`, and `Ctrl+Shift+C` for `terminal.copy`. During keyboard
capture the framework resolves Ctrl/Alt/Meta chords against registered
shortcuts *before* the pane's capture handler, so these chords never reach
`handle_editor_key`; `dispatch_terminal_paste` then fails its
`pty_manager.has(pane)` check and toasts "paste failed: no terminal in
focus". The editor's own `Ctrl+V` arm is dead code.

Fix: `dispatch_terminal_paste` and `dispatch_terminal_copy` check the active
pane's kind first. For an editor pane they paste into / copy from the
buffer (same normalisation as typed input: CRLF → LF, tabs kept) and return
`true`. Read-only panes (diff) accept copy and ignore paste. Regression
tests drive the *dispatch strings*, not the key handler.

### 2. Syntax highlighting (editor and diff)

The Flow snippet tokenizer (`flow_explorer::highlight`) moves to a crate
module `src/syntax.rs`, re-exported from `flow_explorer` so existing paths
keep compiling. It gains:

- a non-allocating span API: `tokenize_spans(line, lang, &mut in_block_comment, out: &mut Vec<Span>)`
  with `Span { start, end, kind }` (byte offsets); `tokenize` stays as a
  wrapper for the snippet renderer;
- more languages, data-only additions: JSON, TOML, YAML, Markdown (plain
  with `#` headings as keywords and fenced code left plain), Shell
  (`sh`/`bash`/`zsh`), PowerShell (`ps1`/`psm1`/`psd1`), C/C++ (`c`/`h`/
  `cc`/`cpp`/`hpp`), C#, Java, Kotlin, Swift, Ruby, PHP, SQL, CSS/SCSS, Lua,
  Zig, Dockerfile and Makefile by basename. Unknown extensions stay `Plain`.

Editor panes paint per-token foreground colours (and BOLD for keywords is
*not* used: attribute changes cost nothing, but keep the look calm). Block
comment state across lines is cached per pane in a `SyntaxState`:
`in_block_comment_at_line_start: Vec<bool>` with a `valid_lines` watermark,
extended lazily up to the last visible line and invalidated from the first
damaged line on every edit. Languages without block comments skip the cache
entirely. Diff documents are immutable, so their state vector is computed
once at load, resetting at every hunk header.

### 3. Colours follow the theme

The editor currently paints white on the theme background. It now takes an
`EditorColors` value derived from the theme's `TerminalPalette`:

| Role | Source |
|---|---|
| text, identifiers | `default_fg` |
| gutter, comments | `ansi[8]` |
| keywords | `ansi[1]` (the `--rust` role) |
| strings | `ansi[2]` (`--sage`) |
| numbers | `ansi[6]` (`--azure`) |
| punctuation | `default_fg` blended 25 % towards `default_bg` |
| selection | `#264f78` (unchanged; identical to the terminal constant) |
| current line | `default_fg` at 6 % alpha (only when nothing is selected) |
| find match / current match | `ansi[3]` at 28 % / 55 % alpha |
| diff added / removed row | `ansi[2]` / `ansi[1]` at 16 % alpha; marker in the solid colour |
| diff hunk header | fg `ansi[6]`, bg `default_fg` at 5 % |
| diff file header | fg `default_fg` BOLD, bg `default_fg` at 8 % |
| diff meta ("\ No newline", binary) | `ansi[8]` |

The renderer alpha-blends cell backgrounds over the pane background, so
tints are stored with alpha rather than pre-blended. Colours are resolved
when a pane is created and again in `mutate_theme` (every open editor is
recoloured and fully repainted once). `EditorColors::default()` equals
today's constants so existing tests keep their assertions. The editor
ignores the Windows Terminal colour-parity switch, as it does today.

### 4. Diff review pane

A **read-only editor pane** whose buffer is a stacked unified diff for one
git range, with per-row decorations. It reuses every editor mechanism:
viewport, wheel patch, mouse selection, copy, find, go-to-line, resize,
close, tab title, persistence stripping. It adds:

- `EditorPane.read_only: bool` — every buffer mutation and `Ctrl+S` become
  no-ops; the dirty marker can never appear.
- `EditorPane.kind: EditorKind::{File, Diff(Box<DiffView>)}` where
  `DiffView { spec: DiffSpec, repo_root: PathBuf, status: Loading | Ready | Failed(String), rows: Vec<DiffRowInfo>, files: Vec<DiffFileInfo> }`.
- `DiffRowInfo { kind: FileHeader | HunkHeader | Context | Added | Removed | Meta | Spacer, file: u32, old_no: Option<u32>, new_no: Option<u32> }`,
  `DiffFileInfo { path (new path, or old path for deletions), old_path, status: Added | Deleted | Modified | Renamed | Binary, first_row, hunk_rows: Vec<usize>, lang }`.

Document layout (one row per buffer line, content rows carry the source
text *without* the leading marker):

```
M src/state.rs                         ← FileHeader (status letter + path; "R old → new" for renames)
@@ -120,7 +120,9 @@ fn dispatch(...)    ← HunkHeader
 120  120      let x = 1;              ← Context   (gutter: old │ new │ marker)
 121      -    let y = 2;              ← Removed
      121 +    let y = 3;              ← Added
                                       ← Spacer between files
```

The gutter shows old and new line numbers (width from the largest number in
the document) and a `+`/`-`/` ` marker column; header rows have an empty
gutter. Syntax colours apply to Context/Added/Removed rows using the file's
language; the row tint sits underneath.

**Loading.** `diff.open:<range>` validates the range, resolves the repo root
(the active workspace's `git rev-parse --show-toplevel`, or the Flow's
`repo_root` for Flow hand-offs), creates the tab immediately with a one-line
"Loading diff <range>…" buffer, then runs git on a named background thread
(`git-diff-<job>`) using `git_command(root)`:

```
git -c core.quotepath=false diff --no-color --no-ext-diff --find-renames -U3 <args> --
```

The result is applied under a brief state lock and followed by a rebuild
request (same shape as `git_watch::resolve_all_in_background`). If the pane
was closed meanwhile the result is dropped. Errors replace the buffer with
the message and toast it; an empty diff shows "No changes for <range>".
Stdout is capped at 8 MiB and the document at 200 000 rows (a trailing Meta
row says it was truncated). CR bytes are stripped before parsing.

**Range rule** (`DiffSpec::parse`), also the contract for Flow hand-offs:

| Input | git args | Meaning |
|---|---|---|
| empty or `HEAD` | `HEAD` | working tree (staged + unstaged) vs HEAD |
| `A` | `A` | working tree vs A |
| `A..B` | `A B` | committed B vs A |
| `A...B` | `A...B` | B vs merge-base(A, B) |
| Flow `diff_range { base, head }` | `head` empty, `HEAD`, `worktree`, or `working tree` → `base`; otherwise `base head` | the producer's default request is "merge base..HEAD including uncommitted changes" |

Any ref starting with `-`, or containing whitespace, `;`, `|`, `` ` `` or a
NUL, is rejected before git runs. Paths are never accepted in v1.

**Navigation inside the pane** (read-only, so plain letters are free):

| Key | Command |
|---|---|
| `n` / `p` | `diff.next_hunk` / `diff.prev_hunk` — cursor to the next/previous hunk header, scrolled to the top of the viewport |
| `]` / `[` | `diff.next_file` / `diff.prev_file` |
| `Enter` / `o` | `diff.open_file` — open the file under the cursor in an editor at its new line number (Removed rows use the hunk's next new line; deleted files toast) |
| everything the editor already has | scrolling, selection, `Ctrl+C`, `Ctrl+F`, `Ctrl+G`, `Ctrl+W` |

Tab title `diff: <range>` (or `diff: HEAD` for the working tree), subtitle
`diff`. Closing never prompts.

**Entry points:** `diff.open` opens a one-line "Diff against…" dialog
(`ConfirmDialog::DiffRequest { buffer, error }`, same card as the Flow
request dialog, prefilled with `HEAD`; a parse error re-opens it inline).
Palette rows "Show uncommitted changes" (`diff.open:HEAD`) and "Diff
against…" (`diff.open`). Global keybind `Ctrl+Shift+G` → `diff.open`.

### 5. Flow hand-offs

In a Flow pane, for the selected node with a `location`:

| Key / control | Command |
|---|---|
| `o`, or the `edit` button beside `src` | `flow.edit:<id>` — open `repo_root/file` in an editor at `line` (focus the existing pane if already open) |
| `d`, or the `diff` button | `flow.diff:<id>` — open the flow's range diff, then scroll to the node's file (and the hunk containing `line` when one exists) |
| the `base..head` chip in the header | `flow.diff` — the whole range |

Node ids contain `::` and `.`, so the id is always the last segment of the
command. Flows without a `diff_range` toast "this flow has no diff range".
The Flow spec's boundary "the app never runs git for this feature" is
amended: the Flow pane never runs git; the diff pane it opens does, on a
background thread, never on the UI thread.

The Ctrl+1/2/3 mode keys documented in the Flow pane are dead today (the
system `tab.switch:N` bindings win). That is logged as a follow-up in
`BACKLOG.md`, not fixed here.

### 6. Navigation

**Open at line, focus existing.** `editor.open_at:<line>[.<col>]:<path>` —
the line comes first because Windows paths contain `:`. `editor.open:<path>`
and `editor.open_at` now look for an editor pane with the same canonical
path in any workspace and focus it (switching workspace/tab) instead of
opening a second copy, then jump. Jumps centre the target line when it is
outside the viewport. (Resolves the BACKLOG "duplicate open" item.)

**Go to line.** Editor-local `Ctrl+G` → `editor.goto` opens
`ConfirmDialog::GotoLine { buffer, error }`; submit runs
`editor.goto:<line>[:<col>]` (1-based, clamped). Both are palette rows /
scriptable.

**Quick open.** Global `Ctrl+P` → `palette.files` opens the command palette
in a new `Files` mode (query prefix `/`, replacing the unused Scrollback
placeholder). Rows come from a per-workspace `FileIndex` built on a
background thread: `git ls-files -z --cached --others --exclude-standard`
from the workspace root, falling back to a bounded walk (skips `.git`,
`node_modules`, `target`, `dist`, `build`, `.venv`; max depth 16, max
50 000 entries) when the root is not a checkout. The index is rebuilt when
it is older than 30 s at the moment the mode opens; the palette shows
"Indexing…" until the first build lands. Matching reuses `fuzzy_match` with
a basename bonus; at most 50 rows are shown (the palette has no
virtualisation) with a footer note when more matched. Selecting a row
dispatches `editor.open:<abs path>`; the allowlist admits the
`editor.open:` prefix because rows only ever come from the index.

**Find in file.** Editor-local `Ctrl+F` → `editor.find` shows a one-row bar
above the grid inside the pane body: a text input (autofocused, seeded with
the single-line selection or the last query), a `3 of 12` counter, `Aa`
toggle, `↑` `↓` `×` buttons. The bar is a framework `Tag::Input`, so while
it has focus keys go to it; `Enter` = next match, `Escape` = system
`modal.close`, whose cascade closes the bar first when one is open. When
the input is removed the framework's focus fallback returns keys to the
capturing grid. With the grid focused, `F3` / `Shift+F3` move between
matches and `Escape` closes the bar. Matching is case-insensitive unless the
query contains an uppercase letter or `Aa` is on; matches are recomputed on
every query change (capped at 10 000, the counter then reads `10000+`), all
visible matches are tinted and the current one is selected. Commands:
`editor.find`, `editor.find_close`, `editor.find_next`, `editor.find_prev`,
`editor.find_query:<text>`, `editor.find_case`.

### 7. Editing polish (last, only if everything above is verified)

Auto-indent on `Enter` (copy the current line's leading whitespace), smart
`Home` (first non-blank, then column 0), `Tab`/`Shift+Tab` with a multi-line
selection indent/outdent by four spaces, `Ctrl+/` toggles the language's
line comment. Each is a pure `EditorBuffer` operation with unit tests; none
touches the framework.

## Commands

All commands go through `state::dispatch` and are therefore usable from
`TM_STARTUP_DISPATCH`. Return `false` when the active pane is not of the
required kind so the key falls through.

| Command | Effect |
|---|---|
| `editor.open:<path>` | open, or focus the existing pane for the path |
| `editor.open_at:<line>[.<col>]:<path>` | open/focus and jump |
| `editor.goto` / `editor.goto:<line>[:<col>]` | dialog / jump in the active editor |
| `editor.find`, `editor.find_close`, `editor.find_next`, `editor.find_prev`, `editor.find_query:<text>`, `editor.find_case` | find bar |
| `diff.open` / `diff.open:<range>` | dialog / diff pane |
| `diff.next_hunk`, `diff.prev_hunk`, `diff.next_file`, `diff.prev_file`, `diff.open_file` | active diff pane |
| `flow.diff`, `flow.diff:<id>`, `flow.edit:<id>` | Flow hand-offs |
| `palette.files` | palette in Files mode |
| `dialog.goto_commit`, `dialog.diff_commit` | dialog submits |
| `terminal.paste`, `terminal.copy` | now editor-aware |

Palette allowlist additions: `editor.goto`, `editor.find`, `diff.open`,
`diff.open:HEAD`, `flow.diff`, `palette.files`, and the `editor.open:`
prefix.

## Keys

| Scope | Chord | Action |
|---|---|---|
| global (rebindable) | `Ctrl+Shift+E` | quick open |
| global (rebindable) | `Ctrl+Shift+G` | diff against… |
| editor pane | `Ctrl+F`, `F3`, `Shift+F3`, `Escape` | find bar |
| editor pane | `Ctrl+G` | go to line |
| editor pane | `Ctrl+V`, `Ctrl+Shift+V`, `Shift+Insert`, `Ctrl+Shift+C` | paste / copy (fixed) |
| diff pane | `n` `p` `]` `[` `Enter` `o` | hunk / file / open |
| flow pane | `o` `d` | edit / diff the selected node |

`Ctrl+Shift+F` stays the FPS overlay toggle; `Ctrl+1..9` stay tab switches.

**Amended during implementation (2026-09-13):** quick open ships on
`Ctrl+Shift+E`, not the `Ctrl+P` this spec first named. Registered chords
are resolved before terminal keyboard capture, so a global `Ctrl+P` would
take readline's "previous command" away from every shell the app hosts —
the same reason `src/keybinds/mod.rs` already moved Open file and Save off
plain `Ctrl+O` / `Ctrl+S`. `Ctrl+Shift+E` is VS Code's Explorer chord and
leaves the terminal alone; the action is rebindable to `Ctrl+P` for anyone
who prefers the editor convention.

## Telemetry

Lifecycle only, never per keystroke, never file content or query text.

`editor-events.jsonl` (existing sink) gains: `editor.goto {path, line}`,
`editor.focus_existing {path, line}` (an open that focused a pane instead
of creating one), `editor.find_open {path}`, `editor.find_closed {path,
line_count}` where `line_count` is the number of hits the session ended
with, `quickopen.index {path: root, line_count: entries, reason:
"git"|"walk"}`, `quickopen.pick {path}`, `editor.paste {path}`.
`editor.open` gains `line` when the open carried a jump target.

The record grew a `line` field distinct from `line_count` so a navigation
target is never recorded where a query would read it as a file size.

`diff-events.jsonl` (new sink, same rotating writer): `diff.request
{job_id, pane_id, range, repo_root, origin: "dialog"|"palette"|"flow"|"dispatch"}`,
`diff.ready {job_id, pane_id, files, hunks, rows, stdout_bytes, elapsed_ms, truncated}`,
`diff.failed {job_id, pane_id, reason: "not_a_repo"|"git_error"|"too_large"|"spawn", elapsed_ms}`,
`diff.rejected {job_id, reason: "bad_range"}` (refused before a pane
exists, so it carries no pane id and never the typed text),
`diff.dropped {job_id, pane_id, reason: "pane_closed"}` (the pane was closed, or its id reused, before the result landed), `diff.open_file
{pane_id, path, line}`, `diff.nav {pane_id, kind: "hunk"|"file"}` (one per
key press is acceptable: these are deliberate navigation actions, not
typing), `diff.closed {pane_id, rows, age_ms}`.

`flow-events.jsonl` gains `flow.handoff {flow_id, kind: "edit"|"diff",
reason: "located"|"no_location"}`.

## Boundaries

- No new crate dependencies. Unified diffs are parsed by hand; fuzzy
  matching reuses the palette matcher.
- No framework (`crates/unshit-framework`) changes. The find bar, focus
  return and key routing all use existing element behaviour.
- git only through `crate::git::git_command`, only on background threads
  for diffs and indexing (the existing synchronous branch/worktree helpers
  are untouched), never with user input as a flag.
- The diff pane is read-only in v1. Editing inside a diff is out of scope.
- Editor/diff panes are not persisted (unchanged).

## Verification

- Unit tests per module (parser fixtures incl. renames, deletions, binary,
  `\ No newline`, CRLF; range parsing and rejection; span tokenizer per
  language; syntax cache invalidation; find matching and wrap; goto
  clamping; index walk caps; palette Files mode ranking and cap).
- State tests through `dispatch` for every command in the table, including
  the clipboard regression, dedupe/focus, dialog error re-open, and the
  diff job apply/drop paths (the job runner is a function the test can call
  synchronously with canned git output).
- Element tests: find bar structure, diff gutter/marker cells, Flow buttons
  and header chip dispatch strings, stylesheet coverage.
- Screenshots via a rewritten `scripts/editor-shot.ps1` (PrintWindow +
  `-Dispatch` chain, isolated profile): highlighted Rust file at a line with
  the find bar open; a diff pane for `HEAD~1`; the quick-open palette.
- Telemetry: trigger each new event once and grep it from the isolated
  profile's sinks.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` (renderer GPU tests with `--test-threads=1`).

## Implementation order

1. Clipboard routing fix (state.rs) — `fix(editor)`.
2. Pure slices in parallel, no shared files: `src/syntax.rs` (+ languages,
   spans); `src/diff/` (parser + document builder); `src/git.rs` additions
   + `src/file_index.rs`; editor internals (`EditorColors`, `SyntaxState`,
   read-only/kind, decorations in `paint_row`, find engine, goto, polish
   buffer ops).
3. Serial wiring in `state.rs` / `ui/`: diff pane + dialog + Flow hooks;
   go-to-line + open-at + dedupe; quick open; find bar UI.
4. Docs (this spec, `SPEC.md` deferrals, Flow boundary, BACKLOG), changelog
   fragments, screenshot script, review pass, screenshots.
