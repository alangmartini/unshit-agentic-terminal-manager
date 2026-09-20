---
name: skill-flow-html
description: Turn a Claude Code or Codex skill (a SKILL.md and the files it references) into an interactive Flow Explorer HTML page - one self-contained file with a call stack, Miller columns and a swim-lane graph of what the skill makes the agent do, step by step, across the human, agent, shell, filesystem, subagent and external lanes. Use when the user asks for the flow of a skill, to visualize or map a skill, "skill flow html", "explain this skill as a flow", "what does /x actually do", or to review what a change to a skill touches.
---

# Skill flow to HTML

You are writing a structured model of **one skill's flow** (what the skill
makes an agent do, from the invocation to the final report) and rendering it
as a standalone HTML page. You do the analysis; the bundled renderer does the
drawing. The page has three views: a collapsible call stack, Miller columns
that drill in, and a swim-lane graph with numbered events.

This is a read-only task against the skill: do not edit it, do not run it.

## Inputs

The user names a skill by slash name (`/create-worktree`), by path, or by
pasting its text. Resolve a name by looking, in order, in
`.claude/skills/<name>`, `.agents/skills/<name>` (project), then
`~/.agents/skills/<name>`, `~/.claude/skills/<name>`, then any
`~/.claude/plugins/*/skills/<name>` and `.codex/skills/<name>`. A name with
a colon (`pkg:name`) is a plugin skill: look under that plugin. If nothing
matches, list what does exist and ask.

Output goes to `flows/<skill-name>.flow.json` and
`flows/<skill-name>.flow.html` under the current working directory unless
the user names another place.

## Steps

1. **Read the whole skill.** `SKILL.md` first, top to bottom, then every
   file it links or names under its directory (`references/`, `scripts/`,
   `assets/`, templates). Note the numbered steps, the headings, every
   command it tells the agent to run, every question it tells the agent to
   ask, every file it writes, every subagent or other skill it invokes, and
   every "if / when / unless" that branches. Record the line numbers as you
   go; the model needs them.

2. **Author the flow.** Follow `references/schema.md` in this skill's
   directory: it holds the schema and the table that maps skill constructs
   (steps, decisions, commands, questions, file writes, subagents, other
   skills) onto nodes, lanes, carriers and edge kinds. Write
   `flows/<skill-name>.flow.json` with:
   * `repo_root` = the skill's directory (absolute path, forward slashes);
   * `entries` = the invocation event in the `human` lane;
   * `edges` in reading order, step 1 before step 2, calls before resolves;
   * a `location` on every step that has a line in the skill text;
   * `next_flow` naming where the flow stops (the next skill, the next
     phase, the task the skill hands back to the user).

3. **Render.** From this skill's directory run

   ```
   node scripts/render.mjs <path to>/flows/<skill-name>.flow.json
   ```

   It validates the document (unknown ids, missing entries, bad kinds,
   unsafe paths), reads each located range out of the skill files and
   embeds it as the row's source excerpt, injects everything into
   `assets/template.html`, and writes the `.flow.html` next to the JSON.
   The last stdout line is a JSON event with the counts and the output path.
   Exit code 2 means validation errors: fix the JSON and rerun. Read every
   `warning:` line. "located line is blank" or "past the end" means a line
   number is wrong; fix it rather than dropping the location.

   If `node` is unavailable, replace the `__FLOW_TITLE__` and `__FLOW_JSON__`
   placeholders in a copy of `assets/template.html` by hand. Inside the JSON
   escape `<` as `\u003c`, `>` as `\u003e` and `&` as `\u0026` (a literal
   `</` in a description would end the script tag), and say that excerpts
   are missing.

4. **Verify.** Open the HTML in a browser tool if one is available and look
   at all three views: the call stack must show every step, the graph must
   show one lane per used process with numbered badges on the event edges,
   and clicking a box must drill into its directly connected steps while staying in the current view. Keep the overview and storyline breadcrumbs visible, with a way to restore the whole graph; reset scrolling when focus changes. Graph boxes, event badges, stack rows and pane items should follow the same progressive navigation model. Source appears only after an explicit right-click → Source action (also available with Shift+F10), never as a side effect of drilling down. Verify back navigation, leaf nodes and source dismissal preserve context. The page opens
   straight into a view from the URL hash, so a screenshot needs no clicks:
   `#view=stack`, `#view=panes`, `#view=graph`, plus `&level=events|code`
   and `&depth=1|2|3|all`. Inspect overview and a decision at both wide and narrow desktop widths: all main steps should be visible in the overview, cards must wrap without truncation, connections must avoid card interiors, and branch choices must be readable together. Keep the whole graph readable by default and offer explicit zoom controls. Without a browser, at least rerun with `--check`
   and confirm zero warnings.

5. **Report** in a few lines: the two output paths, node and edge counts,
   the lanes used, and the `next_flow` sentence. Do not paste the JSON.

## Writing rules

* One sentence per `description`, present tense, saying what the node does
  *for this flow*: "Validates the branch name and resolves the destination
  to an absolute path." Not a restatement of the heading, not a list.
* Name events after what the reader would grep for: the command
  (`git worktree add`), the tool (`AskUserQuestion`), the file
  (`.claude/cycle-state.json`), the slash command (`/spec`). Put what
  crosses the boundary in `payload`.
* Ids are stable: `SKILL.md::<step-slug>` for steps, dotted names for
  events (`ui.invoke`, `tool.bash.git-worktree-add`,
  `tool.bash.git-worktree-add.resolves`). No `;` or newlines.
* Every decision is an `in_memory` event named as its condition, with each
  branch `handled_by` it. Keep both branches; if one is trivial, give it a
  single step with `hidden_children`.
* Every command the skill runs is a `process` event handled by a `shell`
  function; its result is an event the shell function `resolves` when a
  later step depends on it.
* Every question to the user or final report is a `ui` event in the
  `human` lane. The final report is usually the last edge and is
  `resolves`, not `calls`.
* Rules and constraints ("never overwrite", "do not stash") become `tags`
  on the step they constrain, or a clause in `summary`. They are not nodes.
* Cap depth at about six hops; 12–40 nodes is the normal size. Prune with
  `hidden_children` rather than omitting silently.
* Another skill the skill invokes is one `function` node named as that
  skill, with `hidden_children` counting its steps, or the `next_flow`.

## Review mode

When the user asks what a change to a skill touches, set `mode: "review"`,
`diff_range: { base, head }`, and a `status` (`added`, `removed`,
`modified`) on the nodes the diff touched, keeping enough `same` context
that the reader sees where the change sits. Run `git diff <base>..<head>`
on the skill's directory yourself.

## Files in this skill

| Path | Role |
| --- | --- |
| `references/schema.md` | Schema, lane vocabulary, and the skill-to-flow mapping table. Read it before authoring. |
| `assets/template.html` | The standalone renderer. Do not edit per flow. |
| `scripts/render.mjs` | Validate, embed excerpts, inject, write the HTML. Node, no dependencies. |
| `assets/example/create-worktree.flow.json` | A finished flow of a real skill, for shape and tone; `create-worktree/` beside it is the skill it describes and `create-worktree.flow.html` the rendered page. Re-render it with `node scripts/render.mjs assets/example/create-worktree.flow.json`. |
