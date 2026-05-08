# Getting Started with Codex Agent Skills

Codex skills are self-contained folders with a required `SKILL.md`. Codex uses the frontmatter metadata to decide when a skill applies, then loads the skill body only when needed.

## Install Skills

Install a single skill on Windows:

```powershell
$dest = "$env:USERPROFILE\.codex\skills"
New-Item -ItemType Directory -Force $dest
Copy-Item -Recurse .\skills\test-driven-development $dest
```

Install all skills on Windows:

```powershell
$dest = "$env:USERPROFILE\.codex\skills"
New-Item -ItemType Directory -Force $dest
Get-ChildItem .\skills -Directory | Copy-Item -Destination $dest -Recurse -Force
```

Install all skills on macOS/Linux:

```bash
mkdir -p "${CODEX_HOME:-$HOME/.codex}/skills"
cp -R skills/* "${CODEX_HOME:-$HOME/.codex}/skills/"
```

Restart Codex after installing or updating skills.

## Use Skills

Start with `using-agent-skills` if you are not sure which workflow applies. Otherwise, name the skill in your request or let Codex choose from the metadata.

Examples:

```text
Use the spec-driven-development skill for this feature.
```

```text
Use test-driven-development to reproduce and fix this bug.
```

```text
Use code-review-and-quality to review the current diff.
```

## Recommended Baseline

For day-to-day coding, install these first:

- `using-agent-skills`
- `spec-driven-development`
- `planning-and-task-breakdown`
- `incremental-implementation`
- `test-driven-development`
- `code-review-and-quality`

Add the rest as your work needs them.

## Skill Loading Strategy

Do not load every skill body into a prompt manually. Codex skills are designed for progressive disclosure:

1. Metadata stays visible for routing.
2. `SKILL.md` loads only when the skill applies.
3. References load only when the active skill needs deeper material.

This keeps context smaller and makes skill selection more reliable.

## Working Artifacts

Some skills may create files such as `SPEC.md`, `tasks/plan.md`, or `tasks/todo.md`. Treat them as living project artifacts while the work is active:

- Commit them when they are useful for team coordination.
- Update them when scope or decisions change.
- Delete them before merge if the project does not want long-lived planning files.

## Plugin Manifest

The repository includes `.codex-plugin/plugin.json` for Codex plugin environments. The manifest points at `./skills/`, so the same skill folders are used whether you install them manually or through a plugin flow.
