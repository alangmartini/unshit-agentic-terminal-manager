---
name: new-feature
description: Use when implementing a new feature, adding significant functionality, or fixing a non-trivial bug in the terminal-manager codebase. Adaptive TDD workflow: a routing pre-phase picks Express, Standard, or Deep based on detected signals (UI surface, perf surface, security surface, persistence, size, ambiguity, prior-art need). Downstream phases only spawn the agents and run the gates the manifest enables. Triggers on "implement X", "add feature Y", "build Z", "ship feature", or explicit invocation. Examples: "implement tab drag reordering", "add scrollback search", "build settings panel", "fix pane focus regression".
---

# New Feature Workflow

Adaptive TDD workflow. Phase 0 produces a **routing manifest** from task signals. Every later phase checks the manifest before spawning agents or running gates, so heavy machinery (prior art, race mode, security review, bench baseline) only runs when a signal demands it.

## Ground rules

1. When spawning multiple agents with no data dependency, send them in ONE message with multiple Agent tool calls.
2. Brief every agent with the **Agent brief template** below. Agents do not see this conversation.
3. Research and review agents write findings to files under `.claude/tmp/<feature-slug>/`. Main Claude reads artifacts, not raw transcripts.
4. Trust but verify. Read the artifacts agents produce, not just their self-reports.
5. Never automate screenshots, cursor moves, or foreground actions. Visual verification is always user-driven (project memory).
6. **Circuit breaker.** If any phase fails the same way twice, stop and escalate. No third retry.

### Agent brief template

Every brief must include: **Goal** (one sentence outcome), **Inputs** (exact paths, issue link, prior artifacts), **Output path** (single file), **Forbidden edits** (files/dirs off limits), **Output format** (structure + word cap), **Failure mode** (what to do if the goal is unreachable).

## Phase 0: Route (pre-phase, always runs)

Main Claude (no subagent) produces a routing manifest at `.claude/tmp/<feature-slug>/route.yaml` from the description plus, if needed, one cheap Glob or Grep to confirm surfaces touched.

### Signals

| Signal | Detection heuristic | Enables |
|--------|--------------------|---------|
| `surface.ui` | Touches `src/ui/`, `assets/styles.css`, user visible behavior | a11y + obs review, visual checklist |
| `surface.perf` | Touches `src/renderer/`, `src/pty/`, render hot path, PTY buffer path | perf review, bench gate |
| `surface.security` | Touches `unsafe`, PTY command input, path handling, config parse, deserialization | security review |
| `surface.persistence` | Touches saved settings, workspace layout, persisted tab order | migration check |
| `size` | LOC estimate: tiny <20, small <100, medium <500, large 500+ | depth starting point |
| `ambiguity` | Two or more plausible designs from the description alone | race mode |
| `prior_art_useful` | New user visible behavior with no close analog already in repo | prior art survey |
| `is_bug_fix` | "fix", "regression", or references an issue as a bug | regression test mandatory, narrower research |

### Depth mapping

Depth sets defaults; individual gates still toggle on signals.

- **Express**: `size = tiny` AND none of `surface.security`, `surface.perf`, `surface.persistence`. Skips research, DoD table, review agents, baselines.
- **Standard**: `size <= medium` AND not Deep. Codebase research only; gated reviews; coverage gate on.
- **Deep**: `size = large` OR any two of `surface.perf`, `surface.security`, `ambiguity`, `prior_art_useful`. Full research including prior art; DoD table; all gated reviews on.

### Manifest format

```yaml
depth: standard         # express | standard | deep
signals:
  surface_ui: true
  surface_perf: false
  surface_security: false
  surface_persistence: false
  size: medium
  ambiguity: false
  prior_art_useful: false
  is_bug_fix: false
research:
  codebase: true
  framework: true
  prior_art: false       # wezterm | ghostty | alacritty | zellij
tests:
  contract: true         # always
  edge: true
  regression: false      # true if is_bug_fix
  e2e: true
impl:
  race_mode: false
reviews:
  security: false
  performance: false
  simplify: true         # always on (CLAUDE.md mandate)
  a11y_obs: true         # true if surface_ui
gates:
  coverage_diff: true
  bench: false           # true if surface_perf
  migration: false       # true if surface_persistence
docs:
  rustdoc: true
  changelog: true        # if changelog/unreleased/ exists
```

### User override

If the user already asked for a specific depth ("do the full review", "just ship it"), set the manifest and proceed. Otherwise present the initial manifest to the user and confirm before Phase 1. The user can flip any flag.

### Issue and branch

Search GitHub for duplicates. Reopen a closed issue if this is a regression. Create an issue if none exists. Branch name: `feat/<issue-number>-<short-desc>` or `fix/<issue-number>-<short-desc>`.

### Preflight

Confirm `.github/workflows/` has the Claude Code review action if `depth != express`. Coverage baseline (`.claude/tmp/cov-baseline.txt`) and bench baseline (`.claude/tmp/bench-baseline/`) are established lazily in Phase 5 if their gate is on and the baseline is missing.

## Phase 1: Research

Spawn only the agents the manifest enables, in ONE message. All write to `.claude/tmp/<feature-slug>/`.

- `research.codebase` (Explore, medium): `research-codebase.md`. Every file, function, state location likely touched. Paths and line ranges. Under 300 words. May recommend escalating depth with evidence.
- `research.framework` (Explore, quick): `research-framework.md`. Unshit APIs to reuse vs gaps. Under 200 words.
- `research.prior_art`: one Explore (thorough) agent per prior art target selected. Typical set is wezterm, ghostty, alacritty; swap alacritty for zellij if the feature is multiplexing specific (tabs, splits, workspaces). Each writes `research-<project>.md`: data structures, approach, tradeoffs. Cite files and lines; use WebFetch if source is not local. Under 300 words each.

**Synthesis rubric** (stop at the first decisive factor):

1. Fit with existing unshit-framework API and terminal-manager idioms.
2. Simplicity and reviewability.
3. Performance on PTY and render hot paths.
4. Feature parity with prior art.

Write the design brief to `design-brief.md`. If research reveals a signal the Phase 0 manifest missed (e.g., the touchpoints include `src/renderer/` after all), update the manifest and flip the corresponding gate on before Phase 5. Never retroactively downgrade a gate.

## Phase 2: Plan and Definition of Done

1. Update the GitHub issue with acceptance criteria as **observable behaviors**, not implementation notes.
2. For `depth = deep`: include a DoD table mapping each AC to the test file(s) that cover it. For `standard` and `express`: a plain bulleted AC list is enough; the PR body carries the mapping later.
3. Use Plan mode to produce the implementation plan. Cite specific files from the design brief.
4. Get plan approval from the user before writing code.

## Phase 3a: Contract test (always sequential)

1. Scaffold production stubs so the contract test compiles: new types, trait methods, function signatures with `unimplemented!()` or typed placeholder bodies. No behavior.
2. Write ONE failing integration test for the AC1 happy path. This locks the API shape.
3. Verify the red: it compiles and fails for the **right reason** (assertion or `unimplemented!()` panic). A type error red is not acceptable; fix the scaffold.

## Phase 3b: Broader tests (gated by manifest)

Only run the writers the manifest enables. If two or more are enabled, use `isolation: "worktree"` and spawn in ONE message.

- `tests.edge` (general-purpose, worktree): `tests/<feature>_edge_cases.rs`. Every edge case AC row.
- `tests.regression` (general-purpose, worktree): `tests/<feature>_regression.rs`. Include `// Reproduces #N` for bug fixes.
- `tests.e2e` (general-purpose, worktree): `tests/<feature>_e2e.rs`. Full user flow.

Merge worktrees one at a time (project memory). For `depth = deep` only: spawn a dedup agent to consolidate duplicate coverage and extract helpers to `tests/common/mod.rs`. For standard depth, skip the dedup ritual; 2 writers produce little overlap.

Main Claude reads the consolidated test files before proceeding.

## Phase 4: Green

- `impl.race_mode = false` (the common case): sequential Red, Green, Refactor per AC. Run `/simplify` after each green, then move to the next AC. Do not batch refactor.
- `impl.race_mode = true`: spawn 2 agents with `isolation: "worktree"`, same plan and tests.

### Race scorecard

Pass criteria (table stakes, any failure disqualifies): all previously failing tests pass, `cargo clippy -- -D warnings` exits 0, `cargo fmt --check` exits 0.

Tiebreakers among survivors:

1. Smaller LOC delta (ties within 10%).
2. Bench delta >5% regression disqualifies, applied only if `gates.bench = true`.
3. Fewer new or modified `unsafe` blocks, applied only if `surface.security = true`.

Cherry pick strong local ideas from losers if they do not reintroduce a failure.

Unit tests live in `#[cfg(test)] mod tests` alongside production code; not parallelized (shared source files).

Do not enter Phase 5 with any failing test. If implementation forced a plan change, update the DoD list and notify the user.

## Phase 5: Review and gates

### Bash gates (sequential: cargo lock)

1. `cargo test`
2. `cargo clippy -- -D warnings`
3. `cargo fmt --check` (on failure: `cargo fmt`, re-run).
4. If `gates.coverage_diff`: establish or reuse `.claude/tmp/cov-baseline.txt`. On first run per repo, create a worktree at the merge base, run `cargo llvm-cov --summary-only`, copy to baseline, remove worktree. Enforce: total line coverage not dropping AND changed line coverage >= 80% (filter `cargo llvm-cov --json` by `git diff --name-only origin/<base>...HEAD` and `git diff -U0` line ranges).
5. If `gates.bench`: establish or reuse `.claude/tmp/bench-baseline/` the same way. Fail on >5% regression without `waivers.md`.

### Review agents (parallel, only enabled ones, ONE message)

- `reviews.security` (general-purpose): `review-security.md`. PTY command injection, path traversal, input validation, secret handling, `unsafe` blocks, resource exhaustion (unbounded buffers, fd leaks), concurrency (deadlocks, async cancellation, TOCTOU on config paths).
- `reviews.performance` (general-purpose): `review-performance.md`. Allocations in hot loops, unnecessary clones, blocking work on render path. Fail if any bench worsens >5% without a recorded waiver (only meaningful when `gates.bench` is also on).
- `reviews.simplify` (invokes `/simplify`): `review-simplify.md`. Reuse, dead code, premature abstractions. Always on.
- `reviews.a11y_obs` (general-purpose): `review-obs-a11y.md`. Tracing and log coverage for new error paths. Keyboard nav, focus management, screen reader labels via unshit a11y roles.

Any real finding triggers a fix and Phase 5 re-runs from the top. Two consecutive failed Phase 5 runs trip the circuit breaker.

Main Claude reads each enabled review artifact before declaring Phase 5 complete.

## Phase 5.5: Migration check (only if `gates.migration`)

Confirm one of: no schema change (asserted in a test), forward migration exists and is covered by a test loading an older fixture, or explicit user approval recorded in the issue.

## Phase 6: Documentation

Spawn only enabled docs agents in ONE message.

- `docs.rustdoc` (general-purpose): doc comments on new public items in `src/` and `crates/unshit-framework/`. One short line unless a non-obvious invariant needs noting.
- `docs.changelog` (general-purpose): only if `changelog/unreleased/` exists. Creates `changelog/unreleased/<issue-number>-<short-desc>.md` per `changelog/TEMPLATE.md` as a placeholder name; Phase 8 renames it to `<PR-number>-<short-desc>.md` (CLAUDE.md mandate).

This phase does not touch CLAUDE.md, `.claude/`, or project memory.

## Phase 7: Visual verification (user driven)

1. Run `cargo run`. Report to the user when the app is up.
2. Present the AC list (or DoD table for deep depth) as a checklist. User exercises each and replies `y`, `n`, or `skip`.
3. Ask the user to also exercise a short regression surface list drawn from Phase 1 research (unchanged features most at risk from this change).
4. Any `n` returns to Phase 4 with the broken AC as a new failing test input. Circuit breaker still applies.
5. Never automate visual interaction.

## Phase 8: Ship

1. Atomic commits in conventional format (`feat:`, `fix:`, `refactor:`, `test:`, `docs:`, `chore:`, `style:`). No Claude attribution (user memory).
2. Push the branch.
3. Open the PR with: `fixes #N` or `refs #N`, AC list with test id mapping, test plan section.
4. Rename the changelog fragment to `<PR-number>-<short-desc>.md` once the PR number is known (CLAUDE.md mandate). Add a `chore:` commit for the rename.
5. Claude Code GitHub Action reviews the PR if configured. Address findings before merge.

## Phase 9: Post-ship

Poll CI on the PR until it finishes. Report failures with job links. Confirm all required checks green and approvals met. After merge, confirm the commit is on the base branch.

## Merging agent-produced branches

If race mode or parallel worktrees were used:

- Merge one branch at a time (project memory). Never batch.
- After each merge: `cargo check`, then visual verification.
- After conflict resolution, confirm all `AppConfig` callbacks are still wired (`on_close`, `on_scale_factor`, `on_cell_metrics`). Silently droppable.
- Never remove the eager PTY spawn in `main.rs`. Load bearing.

## Express shortcut (when manifest says `depth: express`)

Keep: failing test, fix, `/simplify`, bash gates (`cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check`), visual check, atomic commit.

Skip: Phase 1, 2 (just list ACs in the commit or PR body), 3b, 4 race mode, 5 review agents, 5.5, 6 (unless `docs.changelog` is required), 9 polling (still push and open the PR).
