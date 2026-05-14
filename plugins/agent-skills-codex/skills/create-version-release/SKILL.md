---
name: create-version-release
description: Creates major, minor, or patch/fix version releases from changelog fragments, repository history, and project version metadata. Use when Codex needs to prepare a new version, bump package manifests, assemble the final changelog, create a release commit or tag, or recover release notes from PRs and commits since the previous version.
---

# Create Version Release

## Objective

Prepare a version in a way that is reproducible, auditable, and consistent with the repository's release process. Prefer existing release automation and changelog fragments. When fragments are missing, reconstruct the release notes from PRs and commits since the previous version commit head.

Do not publish packages, push tags, deploy, or mark a release as shipped unless the user explicitly asks for that action.

## Inputs

Use the best available evidence:

- User-requested version kind: `major`, `minor`, `patch`, `fix`, prerelease, or exact version.
- Changelog fragments created by `create-changelog-fragment`.
- Existing changelog, release notes, release docs, and prior version entries.
- Version source of truth: package manifests, lockfiles, build metadata, release config, tags, or release branches.
- PRs, merge commits, conventional commits, issue references, and recent implementation summaries.
- Test, lint, build, security, and ship-review evidence for the release range.

If the requested version kind conflicts with the detected changes, stop and explain the mismatch. Never silently downgrade a breaking change into a minor or patch release.

## Workflow

### 1. Inspect Release Conventions

Read the repository before changing files:

- Version files: `package.json`, `package-lock.json`, `pnpm-lock.yaml`, `yarn.lock`, `pyproject.toml`, `setup.cfg`, `Cargo.toml`, `Cargo.lock`, `go.mod`, `.csproj`, `*.gemspec`, `gradle.properties`
- Release config: `.changeset/`, `changeset`, `release-please-config.*`, `.releaserc*`, `semantic-release`, `towncrier.toml`, `bumpversion`, `hatch`, `poetry`, `cargo-release`, GitHub Actions release workflows
- Changelog locations: `CHANGELOG.md`, `CHANGELOG.*`, `RELEASE_NOTES.md`, `docs/releases/`, `changelog.d/`, `changes/`, `newsfragments/`
- Prior releases: Git tags, release branches, previous changelog headings, package registry metadata when the repo already uses it

Use the repository's release tool when one exists. Prefer `--dry-run`, `--no-publish`, or equivalent modes before writing.

### 2. Establish the Release Range

Find the previous version commit head in this order:

1. The latest version tag matching the repository's tag pattern, such as `v1.2.3`, `1.2.3`, or package-scoped tags.
2. The previous changelog version heading that maps to a commit or tag.
3. The most recent version-bump commit for the relevant manifest or release file.
4. A user-provided base commit.

Resolve tags to commits before comparing:

```bash
git rev-list -n 1 <previous-version-tag>
```

Then compute the candidate range:

```bash
git log --oneline <previous-version-commit-head>..HEAD
git diff --stat <previous-version-commit-head>..HEAD
```

For monorepos, determine whether the repo uses fixed versioning or independent package versions. Only include packages affected by the release range unless the configured release tool says otherwise.

If no reliable previous version commit head exists, ask for the base commit before modifying version files.

### 3. Choose the Version

Respect the user's requested kind when provided:

- `major`: breaking changes, removed APIs, incompatible data migrations, required manual migration, or behavior changes that existing users must adapt to.
- `minor`: backward-compatible features, new public APIs, optional capabilities, or meaningful user-visible improvements.
- `patch` or `fix`: backward-compatible bug fixes, documentation corrections tied to behavior, security patches without API breakage, or operational fixes.

If the user did not choose:

- Infer from changelog fragments, conventional commit markers, PR labels, and diff evidence.
- Treat `BREAKING CHANGE`, `!` conventional commits, removals, incompatible migrations, or documented deprecations as at least `major`.
- Treat `feat` or equivalent user-visible additions as at least `minor`.
- Treat `fix`, `perf`, `docs`, `test`, `chore`, and dependency maintenance as `patch` unless the diff proves broader impact.
- Ask before a `major` release if the evidence is ambiguous.

For prereleases, calendar versions, or non-SemVer projects, follow the repository convention exactly and state the mapping from requested kind to actual version string.

### 4. Assemble the Changelog

Prefer changelog fragments:

1. Collect all unreleased fragments in the repository's configured fragment location.
2. Validate that every release-relevant fragment is represented once.
3. Convert fragments into the repository's established changelog categories and order.
4. Move, delete, or archive consumed fragments only if the existing release tool or convention does that.

If fragments are missing or incomplete, recover entries from history:

1. List PRs merged in `<previous-version-commit-head>..HEAD` using merge commits, commit trailers, GitHub/GitLab CLI output when available, or remote metadata already configured for the repo.
2. Group commits by PR, issue, package, or subsystem to avoid duplicate release notes.
3. Use conventional commits and file-level evidence to classify entries.
4. Read relevant diffs for vague commits before writing user-facing text.
5. Mark uncertain entries as `Needs confirmation` rather than guessing.

Write the final changelog so it is release-ready:

- Add a heading for the new version and release date if the repo convention includes dates.
- Keep any `Unreleased`, `Next`, or between-version section available for future work.
- Lead each bullet with user, operator, integrator, or maintainer impact.
- Include migration, config, feature flag, security, and compatibility notes when supported by evidence.
- Preserve issue, PR, and commit references when the repo style uses them.
- Do not include every file changed, speculative claims, or marketing language.

### 5. Bump Version Metadata

Update version files using the repository's release tool when available:

- Node: prefer `npm version --no-git-tag-version`, `pnpm version`, `yarn version`, Changesets, release-please, or semantic-release config conventions.
- Python: prefer project tooling such as Poetry, Hatch, Flit, bumpversion, or release config; otherwise update `pyproject.toml` or configured source-of-truth files consistently.
- Rust: update `Cargo.toml` and lockfiles as required by the workspace convention.
- Go: prefer tags for module versioning; update files only when the repo has explicit version metadata.
- Monorepos: respect fixed vs independent version policy and workspace release tools.

Keep version metadata, changelog, and consumed fragments in one coherent release-prep change. Do not modify unrelated dependency versions or regenerate broad lockfile sections unless the release tool requires it.

### 6. Verify

Before finalizing:

- Re-run the release tool in dry-run or validation mode when available.
- Run changelog generation or formatting checks when configured.
- Run project validation relevant to a release: tests, build, typecheck, lint, package checks, or the documented release checklist.
- Check that version strings agree across manifests, changelog, tags-to-create, and generated artifacts.
- Check `git diff` for unrelated changes, secrets, accidental generated output, and duplicate changelog entries.

If verification fails, fix the root cause or leave the release uncommitted with a clear blocker.

### 7. Commit and Tag

Create a release-prep commit only when the project expects Codex to commit:

```text
chore: prepare version <version>
```

Create a local annotated tag only when the user asked for it or the repo release process requires it:

```bash
git tag -a v<version> -m "Release v<version>"
```

Do not push commits or tags, publish artifacts, create GitHub releases, or deploy unless explicitly requested.

## Output

Report:

- Previous version commit head and release range.
- Selected version kind and resulting version.
- Changelog source: fragments, PRs, commits, or mixed.
- Files changed and fragments consumed.
- Verification commands run and results.
- Any blockers, assumptions, or `Needs confirmation` entries.
- Whether a commit or local tag was created.
