# Flow document schema and the skill-to-flow mapping

A flow is one JSON document. `entries` + `edges` (array order = reading
order) are the single source of truth; the call stack, the Miller columns and
the swim-lane graph are all derived from them. `scripts/render.mjs` validates
the document, embeds source excerpts, and injects it into
`assets/template.html`.

## Envelope

| Field | Type | Notes |
| --- | --- | --- |
| `schema_version` | `1` | Required. |
| `title` | string | The flow as the reader names it: `"/create-worktree"`, `"Run the /cycle workflow"`. |
| `summary` | string | Two or three sentences: who starts it, what crosses which boundary, where it ends. |
| `repo_root` | string | Directory every `location.file` is relative to. Absolute, or relative to the directory the JSON is written in. For a skill this is **the skill's directory**. |
| `git_ref` | string or null | Informational, shown in the header: `"<skill-name>@<version or short sha>"`. Omit if unknown. |
| `mode` | `"explain"` or `"review"` | `explain` describes the skill as it is; `review` adds per-node diff status. |
| `diff_range` | `{ "base", "head" }` or null | Review mode only. |
| `error` | string or null | Set this (with empty arrays) when the flow cannot be produced; the renderer shows the reason. |
| `next_flow` | string or null | One sentence naming where this flow stops and what continues it. |
| `processes` | array | Swim lanes, see below. Declaration order fixes lane order and colour. |
| `nodes` | array | See below. |
| `edges` | array | Ordered. See below. |
| `entries` | array of node ids | The root event(s). At least one. |

## Processes (lanes)

```json
{ "id": "agent", "label": "Agent" }
```

Use this fixed vocabulary so every skill flow reads the same way. Declare
only the lanes the flow uses, in this order; a lane with no box is not drawn.

| id | label | Holds |
| --- | --- | --- |
| `human` | Human | The invocation, answers to questions, approvals. Usually only events. |
| `agent` | Agent | Steps the model performs itself: reading, deciding, writing, reporting. |
| `harness` | Harness | The CLI runtime: tool dispatch, permission prompts, hooks, skill loading, worktree tools. |
| `shell` | Shell | Commands run through Bash / PowerShell (`git`, `npm`, `cargo`, scripts). |
| `fs` | Filesystem | Files and directories the skill reads or writes as durable artifacts; `state` nodes live here. |
| `subagent` | Subagents | Agents the skill spawns (`Agent` tool, `codex exec`, `claude -p`). |
| `external` | External | Web fetches, MCP servers, APIs, package registries, git hosting. |

Colours cycle through six slots in declaration order, so keep the order above
even when a middle lane is absent.

## Nodes

```json
{
  "id": "SKILL.md::step-3-worktree-add",
  "name": "git worktree add",
  "kind": "function",
  "process": "shell",
  "description": "Creates the branch and the sibling worktree from the current HEAD and checks the exit status.",
  "tags": ["branch", "destination"],
  "location": { "file": "SKILL.md", "line": 9, "end_line": 9 },
  "hidden_children": 0,
  "status": "same"
}
```

| Field | Type | Notes |
| --- | --- | --- |
| `id` | string | Unique. No `;` or newlines. `SKILL.md::<step-slug>` for steps, `references/x.md::<slug>` for steps that live in a reference, `scripts/y.mjs::<function>` for script functions; dotted names for events (`ui.invoke`, `tool.bash.git-worktree-add`, `tool.bash.git-worktree-add.resolves`). |
| `name` | string | What the reader would grep for. Steps: an imperative phrase or the step's own heading. Events: the command, tool, question or file name. |
| `kind` | `function`, `event`, `state` | See mapping below. |
| `process` | process id or null | Required for `function` and `state`. Events that cross lanes may omit it; events that start the flow set it to `human`. |
| `carrier` | see below | Events only. |
| `description` | string | **One sentence**, present tense, what this node does *for this flow*. Not a docstring, not a list. |
| `tags` | string[] | Short names of arguments, flags, state fields or files worth a chip (`slug`, `branch`, `--auto`, `state file`). Three or fewer. |
| `location` | `{ file, line, end_line? }` | Where the instruction lives, relative to `repo_root`, forward slashes, 1-based lines that really hold that text. |
| `payload` | string | Events: what crosses the boundary (`"branch, absolute destination"`, `"{ question, options[] }"`). |
| `hidden_children` | integer | How many callees you pruned; rendered as `[+n]`. |
| `status` | `same`, `added`, `removed`, `modified` | Review mode only; default `same`. |

`render.mjs` adds an `excerpt` object to every located node at render time
(the located lines plus three lines of context). Do not write it by hand.

## Edges

```json
{ "from": "ui.invoke", "to": "SKILL.md::step-1-identify-repo", "kind": "handled_by" }
```

| kind | From → to | Use for |
| --- | --- | --- |
| `handled_by` | event → function | A trigger or an incoming event and the step that acts on it: the invocation → step 1, a tool result → the step that reads it, a user's answer → the branch that follows. |
| `calls` | function → function, or function → event | A step and its substep, or a step and the event it emits (a tool call, a question, a spawn, a file write). |
| `resolves` | function → event | A step and the reply event that closes an earlier request: a command's result, a subagent's report, the final report to the user. Drawn dotted. |

Array order is the order the reader walks: emit the edges of step 1 before
those of step 2, and inside a step emit its calls top to bottom as the skill
text lists them. Cycles are allowed (a retry loop): the tree shows the second
visit as `↩ shown above`.

## Mapping a skill onto the model

| Skill construct | Node kind | Lane | Carrier / edge |
| --- | --- | --- | --- |
| The user invokes the skill (`/name`, "do X") | `event` | `human` | carrier `ui`; it is the entry; `handled_by` the first step. |
| A numbered step, a heading, an imperative paragraph | `function` | `agent` | Parent step `calls` it. |
| A decision ("if the repo is dirty…", "when omitted…") | `event` | (none) | carrier `in_memory`, named as the condition; the deciding step `calls` it; each branch step is `handled_by` it. Keep both branches. |
| A shell command the skill tells the agent to run | `event` | (none) | carrier `process`; the step `calls` it; a `shell` function (the command) is `handled_by` it. Say what comes back in that function's description. Model the result as its own event (the shell function `resolves` it) only when a later step branches on it; give that event the `process` of the lane that receives it so it draws as a pill there. |
| A sequence of numbered steps | `function` each | `agent` | One root function named after the skill's H1 `calls` each step in order; steps are siblings, never a chain (step 1 does not call step 2). Substeps and tool events nest under their step. |
| Reading a file, grepping, listing a directory | `event` | (none) | carrier `fs`; keep it as one event unless the read decides something. |
| Writing a file, a state file, a report, a commit | `event` + optional `state` | `fs` | carrier `fs`; the written artifact is a `state` node in the `fs` lane when later steps read it back. |
| Asking the user a question, a permission prompt, a confirmation | `event` | `human` | carrier `ui`; the answer is a second event the human lane `resolves`. |
| Spawning a subagent, `claude -p`, `codex exec` | `event` | (none) | carrier `process`; the subagent's work is a `function` in the `subagent` lane; its report is an event it `resolves`. |
| A web fetch, an MCP tool, an API, `gh pr create` | `event` | (none) | carrier `http` for HTTP/APIs, `network` for MCP servers; `external` lane holds the function that answers if you model it. |
| A hook, a tool dispatch, a permission gate, `EnterWorktree` | `function` | `harness` | The step `calls` an `rpc` event that the harness function is `handled_by`. |
| Invoking another skill | `function` with `hidden_children` | `agent` | Name it as the skill (`/spec`), describe it in one sentence, count its own steps in `hidden_children`; or stop and name it in `next_flow`. |
| A rule, a "never do X", a constraint paragraph | `tags` on the step it constrains | | Do not make rules nodes; a rule with no step becomes a tag on the nearest step or a clause in `summary`. |
| The final report / summary to the user | `event` | `human` | carrier `ui`; the reporting step `resolves` it. It is usually the last edge. |

Carriers: `ui` (a person types, answers, approves), `process` (a spawned
command or agent), `fs` (a file read or write), `http` (web / API), `network`
(MCP server), `rpc` (a harness tool call that resolves later), `ipc` (a
message between two running processes), `in_memory` (a decision or state
change inside the agent).

## Depth and size

* Cap the depth at about six hops from the invocation. A flow of 12–40 nodes
  reads well; above 60 split it and use `next_flow`.
* When a step fans out into many substeps that do not matter, keep the ones
  that do and count the rest in `hidden_children`.
* Multi-mode skills (`/cycle` with phases, a skill with `explain` and
  `review` modes): model the main path, and put the alternates in
  `next_flow` or under a decision event with `hidden_children` on the branch
  you do not expand.

## Review mode

`mode: "review"`, `diff_range: { base, head }`, and a `status` on every node
the change touched (`added`, `removed` with no `location`, `modified`), plus
enough `same` context that the reviewer sees where the change sits. The
renderer adds `+ - ~` markers, coloured rails and a `diff` legend row.

## Minimal valid document

```json
{
  "schema_version": 1,
  "title": "/example",
  "summary": "The user invokes /example; the agent reads the input and reports.",
  "repo_root": "C:/Users/me/.agents/skills/example",
  "mode": "explain",
  "next_flow": null,
  "processes": [
    { "id": "human", "label": "Human" },
    { "id": "agent", "label": "Agent" }
  ],
  "nodes": [
    { "id": "ui.invoke", "name": "/example", "kind": "event", "process": "human", "carrier": "ui", "description": "The user types the slash command." },
    { "id": "SKILL.md::read-input", "name": "Read the input", "kind": "function", "process": "agent", "description": "Reads the file the user named.", "location": { "file": "SKILL.md", "line": 8 } },
    { "id": "ui.report", "name": "Report", "kind": "event", "process": "human", "carrier": "ui", "description": "One line naming the result." }
  ],
  "edges": [
    { "from": "ui.invoke", "to": "SKILL.md::read-input", "kind": "handled_by" },
    { "from": "SKILL.md::read-input", "to": "ui.report", "kind": "resolves" }
  ],
  "entries": ["ui.invoke"]
}
```
