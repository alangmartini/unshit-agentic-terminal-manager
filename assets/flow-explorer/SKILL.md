---
name: flow-explorer
description: Visualize a concept, explanation, process, code flow, or change as an interactive flow in Unshit Terminal Manager. Use when the user asks for a flow diagram or wants to visualize what you are explaining. Produces Flow Explorer JSON, not HTML or Mermaid.
---

# Flow Explorer producer

Turn **one coherent explanation or flow** into a structured model the user
can navigate as a call stack, Miller columns and a swim-lane graph. The app
renders the JSON you write; you supply the explanation and relationships.

Creating a diagram does not require editing source code, committing, or
running a build. For code flows, analyze the repository you were started in.
For concepts, use the current conversation; a repository is not required.
Preserve the user's broader task if they ask for a diagram during other work.

## Modes

**concepts and explanations** — use `mode: "explain"`. If the user says
"visualize that", use the explanation already in the conversation. Map its
steps, causes, decisions, and outcomes. Use `processes` for actors or logical
lanes, `function` nodes for operations (they need not be real functions),
`event` nodes for triggers or messages, and `state` nodes for conditions or
results. Represent branch outcomes as named events (for example, `Cache hit`
and `Cache miss`) so their meaning is visible in each view. Use the existing
edge kinds to connect these steps. Set `repo_root` to `"."`,
omit `git_ref`, `diff_range`, and `location`, and give every visible step a
process. Only use a `carrier` when it has a real meaning in the explanation.
Do not invent files, line numbers, or code symbols. Label assumptions in the
summary and keep the diagram grounded in the explanation the user requested.

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
- `kind` is `function`, `event` or `state`. Events may carry a `carrier`
  (`ui`, `ipc`, `rpc`, `http`, `fs`, `process`, `network`, `in_memory`).
  Functions carry a `process` id from `processes`.
- Ids are stable and unique: short dotted names for conceptual steps;
  `<file basename>::<Symbol>` for code functions
  (`Editor.tsx::handleKeyDown`), a dotted name for events
  (`rpc.sessions.prompt`, `rpc.sessions.prompt.resolves`).
- Source-backed nodes only: `location.file` is relative to `repo_root`, forward slashes, no `..`;
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
to the output path you were given. For a standalone conversation with no
specified path, use the default output directory in the Local installation
section below, or `~/.unshit/flows/` if there is no such section. Create that
directory if needed and choose a new descriptive filename with a unique
suffix, such as `cache-lookup-<uuid>.json`; preserve existing diagrams.
Write to `<path>.tmp` first, then rename to `<path>`, so a half-written file
is never picked up.

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

## Open the result

For an app-launched task with an explicit output path, the app picks up the
file automatically. Say it was written in one line and stop.

For a standalone conversation, invoke the application executable from Local
installation below, or `terminal-manager` on PATH, with arguments
`flow open <absolute-output-path>`. Quote paths as individual arguments for
the current shell (PowerShell needs `&` before a quoted executable path).
This opens the flow in the running app through local IPC; it does not start
another UI. Keep inherited Terminal Manager environment variables intact.
Check the exit code before saying the flow opened. If the app is unavailable,
keep the JSON, give its full path, and tell the user to choose **Open flow…**
in Unshit's command palette. Do not claim the visualization opened on failure.
Continue the conversation after providing the diagram when appropriate.
