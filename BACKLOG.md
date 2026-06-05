# Unshit Terminal Manager Backlog

## Product ideas

- [ ] **Learning mode using agent skills**
  - **Source idea:** https://gist.github.com/ThariqS/1389dcdff9eba4789887a2211370f06b
  - **Goal:** Add an interactive teaching mode where an agent skill guides the user through code understanding instead of only executing commands.
  - **Core flow:**
    - Start from a task, file, commit, branch diff, or PR.
    - Ask the user to restate current understanding before explaining.
    - Open related files in Unshit as the explanation progresses.
    - Explain code paths, ownership boundaries, data flow, edge cases, and why the implementation exists.
    - Maintain a running Markdown checklist of concepts the user should understand.
    - Quiz with open-ended or multiple-choice questions before moving to the next stage.
    - Keep the session active until the user demonstrates understanding of the checklist.
  - **PR mode:**
    - Ingest a PR diff and identify changed files, entry points, and behavior changes.
    - Walk the user through problem, solution, design decisions, risks, tests, and likely impact.
    - Link each explanation step to the relevant file and code path.
    - End with a review-ready summary and remaining questions.
  - **Implementation notes:**
    - Model this as a reusable agent skill, not hard-coded teaching prompts.
    - Prefer existing terminal/session primitives; avoid blocking IPC in render paths.
    - Treat file opening and navigation as app-level behavior.
    - Store generated learning notes under a predictable workspace path.
  - **Open questions:**
    - Should learning mode run inside Quick Prompt, a dedicated command palette action, or a separate sidebar view?
    - Should mastery checks use agent-native question tooling when available, or an Unshit-native question UI?
    - How should PR mode fetch PR data: local branch diff only, GitHub CLI, or hosted provider API?
