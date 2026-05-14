---
name: create-changelog-fragment
description: Creates release-ready changelog fragments from completed implementation work. Use after a ship review, before a release, or when Codex needs to summarize code changes into an Unreleased or between-version changelog entry using the repository's existing changelog conventions.
---

# Create Changelog Fragment

## Objective

Create a small, factual changelog artifact for the work that has just been implemented and reviewed. The changelog should help future release work assemble the next version notes without re-reading the full diff.

## Inputs

Use the best available evidence:

- The spec, task plan, todo list, and acceptance criteria.
- The ship decision, review reports, rollback plan, and known risks.
- The current branch diff, staged diff, or recent commits.
- Test, lint, typecheck, build, migration, and rollout evidence.
- Issue, PR, ticket, or user request identifiers if present.

Do not invent user impact, compatibility promises, migration steps, or deployment status. If the evidence is unclear, write a conservative note or leave a short `Needs confirmation` item.

## Workflow

### 1. Discover the Repository Convention

Inspect the repo before writing:

- `CHANGELOG.md`, `CHANGELOG.*`, `RELEASE_NOTES.md`, `docs/releases/`
- `.changeset/`, `changeset`, `changelog.d/`, `changes/`, `newsfragments/`
- `towncrier.toml`, `pyproject.toml`, `release-please-config.*`, `.github/release-drafter.*`
- Contribution docs, release docs, package metadata, and recent PR patterns

Prefer the existing convention over introducing a new one.

### 2. Choose the Artifact

Use this order:

1. Existing fragment system: add a new fragment in the existing fragment directory and format.
2. Existing `Unreleased`, `Next`, or `Between versions` section: update only that section.
3. Existing release-notes draft area: add a draft note there.
4. No convention: ask before introducing a changelog structure unless the user explicitly requested autonomous creation. If autonomous creation is required, create `changelog.d/unreleased/<date>-<slug>.md` and document that it is the first fragment for the pending release.

Never edit historical released version sections except to fix a mistake in the current task.

### 3. Write the Entry

Group content using the repository's existing categories. If none exist, prefer Keep a Changelog categories:

- `Added`
- `Changed`
- `Deprecated`
- `Removed`
- `Fixed`
- `Security`

For each item:

- Lead with user-visible or operator-visible impact.
- Mention migrations, config changes, feature flags, rollout notes, and rollback considerations when relevant.
- Include issue, PR, commit, or task references if the repo style uses them.
- Keep wording concise and release-note ready.

Avoid:

- Listing every file changed.
- Internal implementation details that do not affect users, operators, integrators, or maintainers.
- Marketing language, uncertain claims, or "minor fixes" without specifics.
- Saying something shipped to production if the evidence only shows implementation or launch readiness.

### 4. Handle Ship Decisions

- For a `GO` decision, write the fragment as release-ready.
- For a `NO-GO` decision, do not update a public release changelog. Instead, create or update a pending internal note near the plan or release draft that records blockers and the intended changelog once fixed.
- If the work is behind a feature flag or staged rollout, say so plainly and include the default flag state when known.

### 5. Verify

Before finishing:

- Re-read the final changelog text against the diff and ship decision.
- Confirm the artifact follows repo naming and formatting conventions.
- Run the repository's changelog formatter or validator if one exists.
- Run the general validation command if the changelog is part of a docs or release workflow.
- Commit the changelog artifact separately from code changes when the project expects atomic commits.

## Output

Report:

- Changelog artifact path.
- Convention detected.
- Summary of entries added.
- Any assumptions or `Needs confirmation` items.
- Verification commands run and results.
