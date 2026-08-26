# Spec Backlog: Eternal Agent Threads

Tracks: user request from 2026-05-18
Branch base: `feat/rust-terminal-manager`
Working branch: `backlog/eternal-agent-threads`
Status: backlog

## Objective

Make agent conversations durable across app and machine restarts.

When the terminal manager launches an agent thread through Quick Prompt or a
future agent action, it should keep enough metadata to find that same
conversation later. On restart, the app should be able to reopen the thread in
a terminal tab, either by attaching to the existing PTY daemon session or by
running the agent's resume command with the saved conversation ID.

The conversation ID must be first class in the app state so the user and future
automation can always refer back to the exact agent thread.

## User Stories

* **U1** I start a Claude or Codex thread from the terminal manager and close or
  restart the app without losing the ability to return to that conversation.
* **U2** On app startup, agent threads that were marked to reopen come back as
  terminal tabs without me hunting for old commands or IDs.
* **U3** If the PTY daemon is still running the original process, the tab
  reattaches to the live session instead of starting a duplicate agent.
* **U4** If the original process exited, I can resume the same conversation with
  the agent's conversation ID from the original worktree.
* **U5** I can copy or inspect the saved conversation ID from the UI when I need
  to reference the thread externally.

## Acceptance Criteria

### F1. Thread Metadata Persistence

* **A1.1** Every managed agent launch gets a durable app-level thread record
  with a stable local thread ID.
* **A1.2** The record stores at least: agent kind, conversation ID when known,
  launch command, resume command when known, worktree or cwd, tab title,
  creation time, last-seen time, last known PTY session ID, and reopen policy.
* **A1.3** Thread metadata persists to a dedicated file near the existing app
  config files, separate from transient pane layout state.
* **A1.4** Missing or malformed thread metadata never prevents the app from
  launching; bad records are skipped with a visible warning or log entry.

### F2. Conversation ID Capture

* **A2.1** The app can associate a conversation ID with each supported agent
  thread.
* **A2.2** Conversation ID capture is adapter based so Claude and Codex can use
  different discovery strategies.
* **A2.3** If the ID is not available immediately, the thread remains pending
  and updates once the adapter discovers it.
* **A2.4** The UI exposes the current ID, or an explicit pending state, without
  blocking normal terminal usage.

### F3. Startup Restore

* **A3.1** On app startup, records with `reopen_on_startup = true` are restored
  as terminal tabs.
* **A3.2** If the saved PTY session ID still exists in `unshit-ptyd`, the tab
  attaches to it and does not spawn a duplicate process.
* **A3.3** If the saved PTY session ID is gone but the conversation ID and
  resume command are known, the tab launches the agent resume command in the
  saved cwd or worktree.
* **A3.4** If neither attach nor resume is possible, the app keeps the thread
  record visible as unavailable and explains what metadata is missing.

### F4. User Control

* **A4.1** Users can mark an agent thread as reopen on startup or manual only.
* **A4.2** Users can close a restored tab without deleting the saved thread.
* **A4.3** Users can explicitly forget a thread record after confirmation.
* **A4.4** Users can copy the saved conversation ID from the thread UI.

### F5. Safety and Correctness

* **A5.1** Restore never runs arbitrary stale command strings without validating
  they belong to a known supported agent adapter.
* **A5.2** Restore runs in the original cwd or worktree only if it still exists.
* **A5.3** If the saved worktree was deleted, the user sees a recoverable state
  instead of a silent spawn in the wrong directory.
* **A5.4** Existing non-agent terminal sessions keep their current behavior.

## Likely Project Structure

New or touched app modules:

```text
src/agent_threads/
  mod.rs          // public surface
  state.rs        // AgentThread, AgentKind, reopen policy, status
  persist.rs      // load/save thread metadata
  adapters.rs     // Claude/Codex capture and resume command adapters
  restore.rs      // startup attach/resume orchestration

src/quick_prompt/spawn.rs
src/state.rs
src/ui/settings.rs or a future sessions/thread panel
src/main.rs
```

The implementation should reuse the existing daemon attach path before adding
new lifecycle APIs.

## Boundaries

* This does not require preserving the full terminal scrollback after daemon
  restart. The agent conversation is the durable source of truth.
* This does not require syncing thread records across machines.
* This does not require auto-pruning old worktrees, although the metadata should
  make a future cleanup tool possible.
* This should not change normal shell tab startup behavior.

## Open Questions

* What is the current supported way to obtain and resume Claude conversation
  IDs from Claude Code?
* What is the current supported way to obtain and resume Codex conversation IDs
  from Codex CLI?
* Should Quick Prompt threads default to reopen on startup, or should the user
  opt in per thread?
* Should restored threads open immediately as tabs, or should startup show a
  lightweight restore list first?
* Should a thread record be tied to an app workspace, to a git worktree, or both?
