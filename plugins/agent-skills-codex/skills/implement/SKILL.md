---
name: implement
description: Trigger a complete flow of implementation using subagents in an autonomous way, from specification through planning, TDD, review, simplification, ship readiness, and changelog fragment creation.
---

User will invoke with "$agent-skills-codex:implement feature to be implemented/fixed"

You will then follow this steps, IN THE EXACT ORDER:

1. First you will ask clarifying questions to the user:

  - The objective and target users
  - Core features and acceptance criteria
  - Tech stack preferences and constraints
  - Known boundaries (what to always do, ask first about, and never do)

Then, Spawn an subagent to invoke the $agent-skills-codex:spec-driven-development skill with these answers./

The subagent should:
"""
Generate a structured spec covering all six core areas: objective, commands, project structure, code style, testing strategy, and boundaries.

Save the spec as SPEC.md in the project root and proceed.
"""

2.  Spawn an subagent to invoke the $agent-skills-codex:planning-and-task-breakdown
The subagent should:

"""
Read the existing spec (SPEC.md or equivalent) and the relevant codebase sections. Then:

Enter plan mode — read only, no code changes
Identify the dependency graph between components
Slice work vertically (one complete path per task, not horizontal layers)
Write tasks with acceptance criteria and verification steps
Add checkpoints between phases
Present the plan for human review
Save the plan to tasks/plan.md and task list to tasks/todo.md.
"""
3. Here, you will start an loop for each task in the plan, while you (main agent) act as an orchestrator:

Spawn an subagent to invoke the $agent-skills-codex:incremental-implementation skill alongside $agent-skills-codex:test-driven-development skill.

The subagent should:
"""
  Pick the next pending task from the plan. For each task:

  - Read the task's acceptance criteria
  - Load relevant context (existing code, patterns, types)
  - Write a failing test for the expected behavior (RED)
  - Implement the minimum code to pass the test (GREEN)
  - Run the full test suite to check for regressions
  - Run the build to verify compilation
  - Commit with a descriptive message
  - Mark the task complete

If any step fails, follow the debugging-and-error-recovery skill.
"""
4. Loop in 3 until ALL tasks in the plan are marked as done.

5. Spawn an subagent to invoke the $agent-skills-codex:code-review-and-quality skill.

The subagent should:
"""
  Review the current changes (staged or recent commits) across all five axes:

  Correctness — Does it match the spec? Edge cases handled? Tests adequate?
  Readability — Clear names? Straightforward logic? Well-organized?
  Architecture — Follows existing patterns? Clean boundaries? Right abstraction level?
  Security — Input validated? Secrets safe? Auth checked? (Use security-and-hardening skill)
  Performance — No N+1 queries? No unbounded ops? (Use performance-optimization skill)
  Categorize findings as Critical, Important, or Suggestion. Output a structured review with specific file:line references and fix recommendations.
"""

6. Spawn an subagent to invoke the $agent-skills-codex:code-simplification skill.

The subagent should:
"""
Simplify recently changed code (or the specified scope) while preserving exact behavior:

Read CLAUDE.md and study project conventions
Identify the target code — recent changes unless a broader scope is specified
Understand the code's purpose, callers, edge cases, and test coverage before touching it
Scan for simplification opportunities:
Deep nesting → guard clauses or extracted helpers
Long functions → split by responsibility
Nested ternaries → if/else or switch
Generic names → descriptive names
Duplicated logic → shared functions
Dead code → remove after confirming
Apply each simplification incrementally — run tests after each change
Verify all tests pass, the build succeeds, and the diff is clean
If tests fail after a simplification, revert that change and reconsider. Use code-review-and-quality to review the result.
"""

7. Now, we will ship. Invoke the $agent-skills-codex:shipping-and-launch skill.

This is a Codex fan-out orchestrator. Run three specialist subagents in parallel against the current change, then merge their reports into a single go/no-go decision with a rollback plan. The specialists operate independently with no shared state and no ordering dependency, which is what makes parallel execution safe and useful here.

Spawn three Codex subagents concurrently with `spawn_agent`. Issue all three subagent requests before waiting for any result. Sequential spawn-and-wait behavior defeats the purpose of this workflow.

Use the default inherited model unless the user explicitly asks for a different model or there is a clear task-specific reason to override it. Give each subagent an explicit persona prompt:

In Claude Code, each call passes subagent_type matching the persona's name field:

- `code-reviewer`: Run a five-axis review of the staged changes or recent commits: correctness, readability, architecture, security, and performance. Output the standard review template with findings ordered by severity and grounded in file/line references.
- `security-auditor`: Run a vulnerability and threat-model pass. Check OWASP Top 10, secrets handling, auth/authz, dependency CVEs, unsafe defaults, and data exposure. Output the standard audit report.
- `test-engineer`: Analyze test coverage for the change. Identify gaps in happy paths, edge cases, error paths, regression risk, and concurrency scenarios. Output the standard coverage analysis.

Recommended Codex agent setup:

- Use `agent_type: "explorer"` when the specialist only needs to inspect and report.
- Use `agent_type: "worker"` only if the specialist is explicitly asked to edit files.
- Set `fork_context: true` if the specialist needs the same conversation context as the main session.
- Keep each delegated task concrete, bounded, and self-contained.

Each subagent prompt must include:

- The persona name.
- The scope to review, such as staged changes, the current branch diff, or recent commits.
- The required output format.
- A constraint that the subagent must not spawn or delegate to other subagents.
- A constraint that it must return only its own report.

Example prompt shape:

```text
You are the code-reviewer specialist for a Codex ship review.

Review the staged changes or, if nothing is staged, the current branch diff against the base branch. Evaluate correctness, readability, architecture, security, and performance.

Do not spawn or delegate to other agents. Return only your report. Include severity, file/line references where available, impact, and concrete remediation.
```

If the harness does not support Codex subagents, invoke each persona prompt separately and treat the outputs as independent reports. The merge phase still works, but the preferred Codex path is true parallel fan-out.

Phase A Constraints

- The three specialists run in parallel. Spawn all three before calling `wait_agent`.
- Specialists do not call each other and do not spawn additional subagents.
- Each specialist gets its own context and returns only its report to the main session.
- The main Codex session owns synthesis, decision-making, and the rollback plan.
- Do not delegate urgent blocking work that the main session needs before it can continue. For this workflow, the three reports are independent sidecar analyses and are safe to run in parallel.

Phase B: Merge in Main Context

Once all three reports are back, the main Codex session synthesizes them. Do not ask a specialist to make the final decision.

Evaluate:

- Code Quality: Aggregate Critical and Important findings from `code-reviewer` plus any failing tests, lint, typecheck, or build output. Resolve duplicate findings across reports.
- Security: Promote any Critical or High `security-auditor` finding to a launch blocker. Cross-reference with the code reviewer's security axis.
- Performance: Pull from the code reviewer's performance axis and cross-check Core Web Vitals or service-level latency/resource constraints when applicable.
- Accessibility: Verify keyboard navigation, screen reader support, focus management, labels, and contrast directly in the main session, or apply the accessibility checklist if the change has a UI surface.
- Infrastructure: Verify environment variables, migrations, monitoring, alerts, feature flags, deploy order, and operational dependencies directly.
- Documentation: Verify README updates, ADRs, changelog inputs, runbooks, and user-facing docs directly. Changelog artifact creation happens in stage 8.

## Phase C: Decision and Rollback

Produce a single output:

```markdown
## Ship Decision: GO | NO-GO

### Blockers (must fix before ship)
- [Source persona: Critical finding + file:line]

### Recommended fixes (should fix before ship)
- [Source persona: Important finding + file:line]

### Acknowledged risks (shipping anyway)
- [Risk + mitigation]

### Rollback plan
- Trigger conditions: [what signals would prompt rollback]
- Rollback procedure: [exact steps]
- Recovery time objective: [target]

### Specialist reports (full)
- [code-reviewer report]
- [security-auditor report]
- [test-engineer report]
```

## Decision Rules

- The three Phase A specialists run in parallel, never sequentially.
- Specialists do not call each other. The main Codex session merges reports in Phase B.
- A rollback plan is mandatory before any GO decision.
- If any specialist returns a Critical finding, the default verdict is NO-GO unless the user explicitly accepts the risk.
- Skip the fan-out only if all of the following are true:
  - The change touches two files or fewer.
  - The diff is under 50 lines.
  - The change does not touch auth, payments, data access, config, environment variables, migrations, deployment, or infrastructure.
- Otherwise, default to fan-out. This workflow is for production-bound changes, so when the blast radius is non-trivial, run the parallel review even if the diff looks small.

## Codex Execution Notes

- Start by inspecting `git status --short` and the relevant diff.
- If reviewing staged work, use `git diff --staged`; otherwise use the current branch diff against the appropriate base.
- Spawn the three specialist subagents before waiting for any one of them.
- While specialists run, the main session may independently verify build, tests, lint, docs, accessibility, infrastructure, and rollback details if those checks do not duplicate delegated work.
- Use `wait_agent` only when the main session needs the reports to complete the decision.
- After reports return, review them for duplicates, false positives, and missing file/line grounding before issuing the final go/no-go.

8. Create a between-version changelog fragment. Invoke the $agent-skills-codex:create-changelog-fragment skill after the ship decision so the completed work has a release-note-ready artifact for the next version.

Spawn a Codex subagent with `agent_type: "worker"` unless the repository's changelog convention is clearly read-only or the ship decision is `NO-GO`.

The subagent should:
"""
Read the spec, task plan, todo list, ship decision, specialist reports, rollback plan, current branch diff, and recent commits. Then:

- Discover the repository's changelog convention before writing.
- Prefer an existing fragment system, Unreleased section, or release-notes draft area.
- Create or update only the pending between-version changelog artifact.
- Summarize user-visible changes, fixes, migration notes, config changes, feature flags, rollout implications, security notes, and compatibility impact when supported by evidence.
- Do not edit historical released changelog sections.
- Do not claim production deployment happened unless the ship evidence says it did.
- If the ship decision is `NO-GO`, do not update a public release changelog; create or update a pending internal note that records blockers and the intended changelog once fixed.
- Run the repository's changelog formatter or validator if one exists.
- Run the general validation command if the changelog is part of docs or release validation.
- Commit the changelog artifact separately when the project expects atomic commits.

Return the changelog artifact path, convention detected, entries added, assumptions, and verification results.
"""

The main Codex session must review the fragment before final handoff. Confirm that the changelog reflects actual behavior and launch status, not implementation details or speculative product claims.
