---
name: flow-explorer
description: Author a Flow Explorer JSON model of one user-facing flow through a codebase (explain a flow, or review the flows a change touches) for the Unshit Terminal Manager Flow Explorer pane.
---

# Flow Explorer producer

You are writing a structured model of **one user-facing flow** through a
codebase so a reviewer can navigate it as a call stack, Miller columns and
a swim-lane graph instead of reading a raw diff. The app renders the JSON
you write; you do the analysis.

This is a read-only task: do not edit repository files, do not commit, do
not run the build. Run the analysis in the repository you were started in.

## Modes

**explain** — the request names a flow ("Send a prompt", "Open a file
from the palette"). Start from the user-facing entry points that match the
request (a key press, a click, a command, an incoming request), follow the
handlers, calls, IPC/RPC hops and state writes outward until the flow
ends, and stop where the *next* flow begins (name it in `next_flow`).
Cap the depth at about six hops; when a function fans out into many
callees that do not matter to this flow, keep the ones that do and count
the rest in `hidden_children`.

**review** — the request is a change: `<base>..<head>`, a branch name, or
blank. Run `git diff <base>..<head>` (default: the merge base of the
current branch and the default branch, up to `HEAD`, *including*
uncommitted changes) and `git log` yourself. Find the flows the change
touches, pick the one that best explains it (or the one the request
names), and set `status` on every node the diff touches: `added`,
`removed` (a function that no longer exists at head; give it no
`location`), `modified`. Include enough untouched context nodes marked
`same` that the reviewer can see where the change sits.

## Optional scaffold: calldiff

If `calldiff` is on PATH, use it as a structural scaffold and correct it
by reading the code:

```
calldiff tree -e <EntryFunction> --locs --format json
calldiff diff <base> <head> --format json
```

If it is not installed, do the analysis by hand. Never fail the task
because calldiff is missing, and never install it.

## Writing rules

- One sentence per `description`, in the present tense, saying what the
  function does *for this flow*: "Trims the draft, clears it, and restores
  it when the prompt fails." Not a docstring, not a list.
- Name events after the message, route, channel or key the reader would
  grep for: `sessions.prompt`, `sessions.prompt resolves`,
  `Cmd/Ctrl+Enter in the composer`. Put what crosses the wire in `payload`
  (`{ sessionId, text } over the MessagePort`).
- `kind` is `function`, `event` or `state`. Events carry a `carrier`
  (`ui`, `ipc`, `rpc`, `http`, `fs`, `process`, `network`, `in_memory`).
  Functions carry a `process` id from `processes`.
- Ids are stable and unique: `<file basename>::<Symbol>` for functions
  (`Editor.tsx::handleKeyDown`), a dotted name for events
  (`rpc.sessions.prompt`, `rpc.sessions.prompt.resolves`).
- `location.file` is relative to `repo_root`, forward slashes, no `..`;
  `line`/`end_line` are 1-based and must point at the real definition.
- `edges` are ordered: the array order is the call order the reviewer
  reads top to bottom. `calls` (function → function or function → event),
  `handled_by` (event → function), `resolves` (function → reply event).
- `entries` lists the root event(s). Every id referenced anywhere must
  exist in `nodes`; every `process` must exist in `processes`.
- `tags` are short service-state or argument names worth a chip
  (`draft`, `readySessionId`); `hidden_children` is the count of callees
  you pruned.

## Output

Write exactly one JSON document (no Markdown fences, no prose around it)
to the output path you were given. Write it to `<path>.tmp` first, then
rename it to `<path>`, so a half-written file is never picked up.

If you cannot produce the flow (no matching entry point, the change is
empty, the repo is not a git checkout in review mode), write the same
envelope with an `error` string and empty arrays instead, so the app can
show the reason.

```json
{
  "schema_version": 1,
  "title": "Send a prompt",
  "summary": "The user presses Cmd/Ctrl+Enter. The text crosses into the main process over one RPC, Pi starts the agent run, and the call resolves when the run ends.",
  "repo_root": "C:/work/halo-v2",
  "git_ref": "feat/prompt-restore@a1b2c3d",
  "mode": "explain",
  "diff_range": null,
  "error": null,
  "next_flow": "The agent run itself is the next flow.",
  "processes": [
    { "id": "outside", "label": "Human" },
    { "id": "renderer", "label": "Renderer" },
    { "id": "main", "label": "Main process" }
  ],
  "nodes": [
    {
      "id": "ui.cmd-enter",
      "name": "Cmd/Ctrl+Enter in the composer",
      "kind": "event",
      "process": "outside",
      "carrier": "ui",
      "description": "plain Enter is a newline"
    },
    {
      "id": "Editor.tsx::handleKeyDown",
      "name": "Editor.handleKeyDown",
      "kind": "function",
      "process": "renderer",
      "description": "Enter with meta or ctrl calls onSubmit.",
      "location": { "file": "apps/electron/src/renderer/main/agent/Editor.tsx", "line": 93, "end_line": 100 },
      "status": "same",
      "tags": [],
      "hidden_children": 0
    },
    {
      "id": "rpc.sessions.prompt",
      "name": "sessions.prompt",
      "kind": "event",
      "carrier": "rpc",
      "description": "resolves when the agent run ends",
      "payload": "{ sessionId, text } over the MessagePort",
      "hidden_children": 1
    }
  ],
  "edges": [
    { "from": "ui.cmd-enter", "to": "Editor.tsx::handleKeyDown", "kind": "handled_by" },
    { "from": "Editor.tsx::handleKeyDown", "to": "rpc.sessions.prompt", "kind": "calls" }
  ],
  "entries": ["ui.cmd-enter"]
}
```

`diff_range` is `{ "base": "main", "head": "feat/x" }` in review mode.
`git_ref` is `<branch>@<short sha>` of the tree you analysed. Keep the
document under 8 MiB (a flow is usually a few KiB).

When the file is written, say so in one line and stop; the app opens it.
