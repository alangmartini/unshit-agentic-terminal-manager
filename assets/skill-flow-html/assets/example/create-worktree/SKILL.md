---
name: create-worktree
description: Create a new isolated Git worktree and continue working there. Use when the user invokes create-worktree or asks to create and switch to a new worktree, with an optional name.
---

# Create worktree

Adapted from the user's Claude `/create-worktree` command. Create a new worktree immediately without routine follow-up questions. An optional name follows the invocation; otherwise generate a short random name.

1. From the current working directory, identify the repository root, current HEAD, and existing worktrees using Git. Remember the original directory for a later return. Read applicable repository instructions.
2. Use the supplied name as a simple directory slug, normalizing spaces and unsafe path characters to hyphens. Never interpret it as a path or shell code. Generate a random slug when omitted. Use branch `codex/<slug>` and a sibling directory `<repository-directory-name>-worktrees/<slug>` outside the source checkout. Validate the branch with `git check-ref-format --branch` and resolve the destination to an absolute path. If either exists, append a short random suffix; never overwrite or reuse an existing worktree.
3. Run `git -C <source-root> worktree add -b <branch> <absolute-destination> HEAD`, quoting each argument correctly for the active shell. Check the exit status before continuing. Branch from the current committed HEAD; leave uncommitted and untracked source files in place, and mention this briefly if the source is dirty. Do not stash, reset, commit, fetch, copy secrets, install dependencies, or launch another agent during worktree creation.
4. Verify the destination with `git -C <destination> rev-parse --show-toplevel` and `git -C <destination> branch --show-current`. Read its applicable instructions. Use this absolute destination as the explicit `workdir` for every subsequent shell call and as the base for file edits, searches, tests, and Git operations. A shell `cd` does not persist across tool calls. Preserve this working-directory choice in any continuation summary.
5. Report concisely:
   - `Path: <absolute path>`
   - `Branch: <branch>`
   - `I'll run subsequent commands in this worktree.`

Do not claim the app's workspace or session metadata changed: this skill changes where the agent performs work, not the UI's project root. Do not promise Claude's `EnterWorktree` or `ExitWorktree` tools. When the user later asks to leave, return to the saved source directory and keep the worktree unless removal is requested. Never automatically remove a worktree when a task finishes.

## Implementation completion

Whenever implementation is completed in this worktree, always open a pull request against the repository's default branch before reporting the task finished. This is the user's standing instruction: commit, push, and create the PR without another routine confirmation. A request only to create a worktree does not require an empty PR.

1. Verify the implementation using the repository's required checks, review the diff, and commit the task changes following repository conventions.
2. Query the hosting service for the repository's current default branch; do not assume `main`, `master`, or the branch from which the worktree was created.
3. Push the worktree branch and open a PR with the default branch explicitly set as its base. If an open PR already exists for this branch, update it instead of creating a duplicate. Describe the final behavior and validation results; follow repository authorship and attribution rules.
4. Verify the PR's URL and base branch, then include the link in the final response. Keep the worktree; opening a PR does not authorize merging it. If publication fails, report the concrete blocker and remaining action rather than claiming the task is complete.
