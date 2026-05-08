# AGENTS.md

This repository is the Codex-only fork of `addyosmani/agent-skills`.

## Repository Purpose

The repo packages software engineering workflows as Codex skills. The durable artifacts are:

- `skills/<skill-name>/SKILL.md` for on-demand Codex skills
- `references/*.md` for optional supporting material
- `.codex-plugin/plugin.json` for Codex plugin metadata
- `docs/` for Codex-specific usage and contribution docs

Do not reintroduce non-Codex packaging such as alternate agent command folders, alternate agent manifests, setup guides for other tools, or agent-specific hook scripts.

## Codex Skill Rules

- Treat each `SKILL.md` as a workflow, not a blog post.
- Keep frontmatter valid and minimal: `name` and `description` are required.
- The skill directory name must exactly match the `name` field.
- Descriptions must say both what the skill does and when Codex should use it.
- Keep `SKILL.md` focused. Move long examples, checklists, or provider-specific details to directly linked reference files.
- Avoid adding README files inside individual skill directories.

## Editing Guidance

- Preserve existing skill behavior unless the task explicitly changes it.
- Use ASCII for new docs unless an existing file clearly requires another character set.
- Prefer small, scoped edits over broad rewrites.
- When adding a new reusable workflow, add it as a skill directory under `skills/`.
- When adding supporting material, place it in `references/` or inside the skill directory only if it is tightly coupled to that skill.

## Validation

Run the validator after changing manifests, skills, or docs:

```bash
python scripts/validate-skills.py
```

The validator checks:

- `.codex-plugin/plugin.json` exists and points at `./skills/`
- every `skills/*/SKILL.md` has required frontmatter
- each skill `name` matches its directory
- removed platform-specific artifacts stay removed

If platform-specific material is intentionally added later, document why it belongs in this Codex-only fork.

## Project Structure

```text
.codex-plugin/
  plugin.json
docs/
  codex-setup.md
  getting-started.md
  skill-anatomy.md
references/
  accessibility-checklist.md
  performance-checklist.md
  security-checklist.md
  testing-patterns.md
skills/
  <skill-name>/
    SKILL.md
scripts/
  validate-skills.py
```

## Release Criteria

Before pushing changes to this branch:

- `python scripts/validate-skills.py` passes
- `git status --short` contains only intentional changes
- docs describe Codex behavior only
- no deleted non-Codex packaging has been restored
