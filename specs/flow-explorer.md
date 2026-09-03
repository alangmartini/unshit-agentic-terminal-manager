# Spec: Flow Explorer

An agent writes a structured model of one user-facing flow through a codebase (the events, handlers, IPC hops and state it touches, each with a one-sentence description and a source location), and the app renders it as a native pane with three views: a call stack, Miller columns, and a swim-lane graph. In review mode every node carries a diff status, so a change is reviewed as the flows it touches rather than as a raw diff.

Reference: the "you and I are not reviewing diffs the same" demo (https://x.com/tanishqk/status/2095277797837283541). The producer side is derived from the same author's `calldiff` and `code-walkthrough` tooling; the renderer is built from the video.

## Objective

Turn "read this diff" into "walk this flow". The agent does the analysis it is good at (naming the flow, describing what each function does for it, tagging processes and carriers); the app does the navigation it is good at (collapsible trees, columns that drill in, a graph that zooms). Nothing about the feature runs on the render path, spawns processes from the UI thread, or touches repository files.

## User stories

* **U1** I open the command palette, pick "flow: explain…", type "Send a prompt", and a tab opens running my default agent in the workspace directory. When the agent finishes, a "Send a prompt" tab appears with the flow, and the agent tab stays so I can ask for the next flow.
* **U2** I pick "flow: review…", leave the range blank (or type `main..HEAD`), and get the same pane with `+`/`-`/`~` rails on the nodes the change touched and a `base..head` in the header.
* **U3** I pick "flow: open…", choose a JSON file an agent wrote elsewhere (or one under the profile's `flows/` directory), and it opens as a pane.
* **U4** In the call stack I collapse subtrees, switch between events / code / source levels, open a highlighted source excerpt under a row, and drive all of it from the keyboard.
* **U5** In the panes view I click an entry event, then a handler, then a callee; each opens a column to the right, older columns collapse to rotated strips I can click to go back.
* **U6** In the graph I see one lane per process, boxes for functions, numbered event badges on the edges; clicking a badge zooms into what its receiver does, clicking a box opens the panes view on that node, and a depth filter bounds the picture.
* **U7** A broken or half-written flow file never opens a pane; I get a toast with the reason and a telemetry line I can grep.

## Acceptance criteria

### F1. Data model and ingestion
* **A1.1** A flow is a JSON document with `schema_version: 1`, `title`, `summary`, `repo_root`, `mode` (`explain` | `review`), optional `git_ref`, `diff_range`, `error`, `next_flow`, and ordered `processes`, `nodes`, `edges`, `entries`. `flow_explorer::model` is the schema; the fixture `tests/fixtures/flow-explorer/send-a-prompt.json` is the reference document.
* **A1.2** Validation collects every error (unsupported schema version, duplicate ids, unknown process/node references, empty entries, unsafe location paths) and a document with any error, or with `error` set, never opens a pane.
* **A1.3** Files over 8 MiB are refused before parsing. Ingestion is shared by the manual open and the launch poller.
* **A1.4** The call stack, columns and graph all derive from `entries` + `edges` in array order; a node reached twice on one path renders once more as a leaf ("shown above") so cycles terminate.

### F2. Pane and views
* **A2.1** `flow.open:<path>` opens a flow as a new tab; the pane is stripped from persisted layouts like editor panes and can be reopened from its file.
* **A2.2** The toolbar shows the six view names from the reference (`stack graph`, `tree`, `call stack`, `panes`, `graph`, `sequence`); the three unimplemented ones render disabled. Levels are `events` / `code` / `source`.
* **A2.3** Call stack rows show connector, name coloured by process, carrier and tag chips, `[+n]` for pruned callees, description, location and a `src` button that toggles a highlighted excerpt (3 context lines, gutter, located range highlighted). Row cells shrink in a fixed order so nothing collapses to zero width.
* **A2.4** Panes view: column 0 is the overview (title, summary, entries); each chosen item opens a column with head (name, kind, process, carrier, tags, description, payload), `calls` / `resolves` / `handled by` / `handles` lists and a `source` section. Columns older than the last two collapse to 28px strips with the name rotated.
* **A2.5** Graph view: lanes for processes that hold a box, boxes for functions (an entry event is a pill), events folded into numbered edges labelled `carrier · name`, dotted `resolves` edges, breadcrumb, hint, `depth: 1 2 3 all`.
* **A2.6** Review mode adds `+` / `-` / `~` markers and coloured rails to rows, items, column heads and graph boxes, `base..head` to the header and a `diff` legend row.

### F3. Commands and keyboard
* **A3.1** Every interaction is a `flow.*` dispatch acting on the active flow pane, so clicks, keys, tests and `TM_STARTUP_DISPATCH` share one path (see Commands).
* **A3.2** The active flow pane owns: Up/Down/PageUp/PageDown/Home/End (row or column cursor), Right/Enter (expand or open), Left (collapse or pop), Space (toggle), `s` (source), `e`/`c` (expand/collapse all), Escape (clear), Ctrl+1/2/3 (views). Plain digits and everything else fall through to global keybinds.

### F4. Producer
* **A4.1** `assets/flow-explorer/SKILL.md` is both the copyable skill and the prompt body; `producer::build_prompt` prepends mode, request and output path. The full prompt is written to `data_dir()/flows/<flow_id>.prompt.md` and the agent's argv is one line pointing at it (`producer::launch_prompt`): the launch goes through `claude.cmd` / `codex.cmd`, and cmd.exe cuts a command line at the first newline.
* **A4.2** `flow.explain` / `flow.review` open a one-line dialog (`ConfirmDialog::FlowRequest`, commit via `dialog.flow_commit`); `flow.explain:<request>` / `flow.review:<range>` launch without it. A launch runs the workspace's default Quick Prompt agent in the workspace cwd (not a worktree, so uncommitted changes are visible) with the output path `data_dir()/flows/<flow_id>.json`, records the pending launch on the agent pane and emits `flow.launch`. Claude launches add `--add-dir <flows dir>` so the one write outside the repository is inside a working directory. Preconditions (a workspace directory; a git checkout for review) fail inline in the dialog or as a toast.
* **A4.3** A background poller checks pending outputs once a second (`poller::poll_once`: deadline, then `metadata`, then a 1.5 s write grace, then ingest), ingests a finished file exactly once, opens the pane (`flow.ready`) or toasts the reason (`flow.parse_failed`), and drops entries when the agent pane closes (`flow.timeout` / `pane_closed`) or after 60 minutes (`flow.timeout` / `deadline`). Nothing runs on the render path; the state lock is held only to snapshot and to apply.

### F5. Telemetry
* **A5.1** `config_dir()/flow-events.jsonl` (rotating) receives `flow.open`, `flow.open_failed`, `flow.close`, `flow.view_changed`, `flow.snippet_load_failed`, `flow.launch`, `flow.ready`, `flow.parse_failed`, `flow.timeout` with `flow_id`, `mode`, `node_count`, `edge_count`, `reason`, `view`, `agent`, `elapsed_ms` as applicable. Never node names, prose or source text.

## Tech stack

Rust, the in-repo `unshit` UI framework (CSS-styled element tree, Taffy layout, wgpu renderer, SVG paths for edges), `serde_json` for the model, `rfd` for the open dialog. No tree-sitter, no `syntect`: the excerpt highlighter is a small hand-rolled scanner (`flow_explorer::highlight`) and is the documented swap point.

## Commands

| Dispatch | Effect |
| --- | --- |
| `flow.open` / `flow.open:<path>` | Open a flow file (picker / direct). |
| `flow.explain` / `flow.review` | Open the request dialog (`dialog.flow_commit` launches). |
| `flow.explain:<request>` / `flow.review:<range>` | Launch without the dialog (startup dispatch, tests). |
| `flow.view:{stack\|panes\|graph}`, `flow.level:{events\|code\|source}` | Switch view / level. |
| `flow.expand_all`, `flow.collapse_all`, `flow.toggle:<row>`, `flow.src:<id>` | Call stack. |
| `flow.select_row:<row>`, `flow.select_move:<n>`, `flow.select_first/last/into/out/none`, `flow.toggle_selected`, `flow.src_selected` | Cursor (rows, or columns in the panes view). |
| `flow.select:<col>:<id>`, `flow.focus:<col>` | Panes view. |
| `flow.graph.zoom:<id>`, `flow.graph.crumb:<n>`, `flow.graph.depth:{1\|2\|3\|all}`, `flow.graph.details:<id>` | Graph view. |

## Project structure

```
assets/flow-explorer/SKILL.md          producer skill / prompt body
src/flow_explorer/
  model.rs      schema, validation, parse_flow
  tree.rs       call-stack derivation (rows, repeats, visible/collapsible)
  ingest.rs     size cap + parse + validate + error envelope
  pane.rs       FlowPane state and every view transition
  snippet.rs    bounded, contained source excerpts
  highlight.rs  line tokenizer per language
  graph.rs      swim-lane layout (pure geometry)
  producer.rs   build_prompt over SKILL.md
  poller.rs     pending launches -> ready / failed / timed out
  telemetry.rs  flow-events.jsonl
src/ui/flow_pane.rs                    the pane body (all three views)
src/ui/icons.rs                        carrier icons
tests/fixtures/flow-explorer/          send-a-prompt.json, send-a-prompt.review.json, repo/
scripts/flow-explorer-shot.ps1         isolated screenshot of any dispatch sequence
```

## Testing strategy

* Unit tests per module (model validation, tree derivation, transitions, snippet containment and caps, highlighter, graph layout determinism/overlap/cycles, prompt size, poll outcomes).
* State tests dispatch the `flow.*` strings against the fixtures and assert pane state and telemetry-relevant transitions.
* UI tests build the pane body and assert element classes/texts for all three views and review mode.
* Screenshots: `scripts/flow-explorer-shot.ps1 -Dispatch "flow.open:<fixture>;flow.view:panes;..."` under a throwaway profile, captured with `PrintWindow`, compared by eye against the reference frames.
* Gates: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test -p terminal-manager` (the workspace's GPU-gated renderer tests are run single-threaded separately).

## Boundaries

* The app never runs git for this feature and never edits repository files; the agent does the analysis in the workspace cwd.
* Snippets are read only from inside `repo_root` (canonicalised containment check), capped at 256 KiB, UTF-8 only.
* No file watcher crate: pending launches are polled once a second by one thread; opened flows are not re-read unless reopened.
* No synchronous IPC or disk reads during render; snippet loads happen on the dispatch path and are cached per node.
* Telemetry never contains node names, prose, paths inside the repo, or source text.

## Open questions

* Should an opened flow reload in place when the agent rewrites the file (mtime re-ingest)? Deferred; `flow.open` again is the workaround.
* Codex: `codex exec` is launched with the same one-line prompt, but its workspace-write sandbox may refuse the write to `data_dir()/flows/`; if it does, a `-c sandbox_workspace_write.writable_roots=[...]` override is the likely fix. Not verified.
* Claude in default permission mode asks once before writing the flow JSON (the write is outside the repository); users who auto-accept edits get no prompt because of `--add-dir`.
* Auto-scroll the panes view to the newest column: needs a scroll-request declaration in the framework.
* The three unimplemented views (stack graph, tree, sequence) stay disabled until there is a reason to build them.

## Decisions log

* 2026-09-03 Native pane, not HTML; three views in v1; app-launched producer with a poller plus a manual open; review mode in v1 (user).
* 2026-09-03 Collapse state keyed by row index, not node id (diamonds and repeats collapse independently).
* 2026-09-03 Events fold into graph edges; an entry event is drawn as a pill so the first edge has a source.
* 2026-09-03 Expanded columns are flexible (200-480px) rather than fixed 320px; a half-width pane cannot fit two fixed columns.
* 2026-09-03 The agent's interactive tab is kept and the flow opens as a new tab.
