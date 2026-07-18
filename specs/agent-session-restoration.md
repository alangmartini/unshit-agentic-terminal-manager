# Spec: Agent Conversation Restoration

## Objective

Restore a terminal pane's Claude Code or Codex conversation after the PTY daemon has been lost, including a machine restart. A normal UI-only restart must continue to attach to the daemon-owned PTY and must never start a second agent.

The durable state is the agent CLI's own conversation store. Terminal Manager persists only the minimum routing and launch metadata needed to select that conversation again: agent kind, stable workspace and pane identity, working directory, provider launch mode and phase, whether Terminal Manager created the pane, opaque conversation id when known, and the time the conversation was observed. Prompts, transcript contents, terminal output, and hook payloads are never persisted or logged.

## User stories

- As a user who closes only the Terminal Manager window, I return to the same still-running Claude or Codex process.
- As a user whose PC restarted, I can press `Resume Claude` or `Resume Codex` in the restored pane and continue the recorded conversation.
- As a user who explicitly enables automatic agent restoration, the recorded conversation relaunches without a click after a confirmed cold start.
- As a user with two agents in the same directory, each pane resumes its own opaque conversation id rather than whichever transcript happens to be newest.
- As a user who chooses `Kill all`, I do not see intentionally killed panes return later.

## Functional requirements

### Durable restart metadata

- Each persisted pane may carry a backward-compatible optional agent restart record.
- The record contains the provider (`claude` or `codex`), working directory, provider launch mode and phase, optional validated conversation id, observation time, and whether Terminal Manager created the agent pane.
- Persisted workspace numeric ids are stable. Removing a workspace never renumbers surviving workspaces or changes the daemon keys used to reconcile their panes.
- Quick Prompt records the provider and target directory before the new pane is considered complete.
- Claude Quick Prompt launches with a caller-generated UUID through `--session-id`, so its exact id is durable immediately.
- Session hooks update the pane record as soon as Claude or Codex reports a new session id. The update is written to `workspaces.json` immediately and does not depend on `on_close` running.
- Prompt text, transcript paths, transcript contents, hook stdin, terminal output, and command histories are excluded from the persisted record.

### Warm and cold reconciliation

- Reattachment to a live daemon session always wins over restoration.
- A resume command may launch only after the daemon has positively confirmed no live session matches `(workspace_id, pane_id)`.
- The initial session list is only a cache optimization. Its failure never proves that a pane is absent and never selects a fallback by itself; the daemon-atomic `EnsureSession` transaction remains the authoritative attach-or-spawn decision.
- A cached attach that returns a typed `NotFound` falls through to `EnsureSession`, which rechecks atomically. A transient cached attach failure surfaces an error and retains the cache entry instead of falling back to a spawn.
- `EnsureSession` attaches the one live matching key or spawns the supplied fallback while holding the registry lock. A lost response is safe to retry because the second request attaches the session created by the first.
- Both `EnsureSession` and ordinary daemon spawn reject a second live session with the same `(workspace_id, pane_id)`, closing races across concurrent or older clients.
- The active startup pane, background restored panes, and deferred dimension-sync panes all use the same spawn-plan resolver.

### Resume behavior

- Exact Claude restore command: `claude --resume <uuid>`, launched in the recorded directory.
- Interactive Codex sessions restore with `codex resume -C <cwd> <uuid>`, passed as structured argv.
- Non-interactive Codex sessions created by Quick Prompt restore with `codex exec resume <uuid>` in the recorded directory.
- With automatic restoration disabled, a confirmed cold miss starts the configured ordinary shell in the recorded directory and overlays a `Resume Claude` or `Resume Codex` button.
- Clicking the button acknowledges and terminates the temporary daemon session, then launches the provider with structured argv through daemon-atomic reconciliation. No command text is injected into shell input. A concurrently recovered or lost-response winner is attached without executing the fallback, enters the same non-clickable confirming phase, and restores the button only if its PTY exits before provider confirmation.
- With automatic restoration enabled, a confirmed cold miss uses the structured resume command as the spawn fallback.
- Immediate spawn failure retains the metadata and manual affordance. The affordance is hidden while a spawned resume waits for a provider hook to confirm it; PTY exit before confirmation restores the button. One pane's failure does not block other panes or app startup.
- The CLI restores conversation history and agent state that the provider persisted. It cannot recreate an interrupted model request, running tool, approval prompt, or background OS process.

### Conversation discovery fallback

- An exact hook or managed Quick Prompt id is the primary signal.
- When a managed agent record has no id, Terminal Manager may read only a bounded prefix (at most 128 KiB and 64 JSONL records) from recent local provider metadata files and offer the matching conversation for that recorded directory.
- Heuristic discovery is restricted by provider, normalized directory, observation time, and a recency window. Ambiguous candidates produce no automatic action.
- Discovery extracts only the allowlisted session id and cwd. It never retains, logs, or persists the remaining record or transcript content.

### Consent and hook management

- Automatic restoration defaults to off.
- Enabling `Resume agent conversations automatically` in the Sessions settings is the explicit consent action for both auto-resume and immediately installing Terminal Manager's managed hook entries.
- The hook merge is idempotent, preserves unrelated user settings, refuses malformed JSON, and uses atomic file replacement.
- The Claude managed entry invokes the current Terminal Manager executable with an exec-form `command` plus `args`. The Codex entry uses provider-native, safely quoted `command` and `commandWindows` fields. Both read at most 64 KiB of provider hook JSON from stdin.
- Each daemon-owned PTY receives an unguessable hook capability. Hook observations must match the trusted profile endpoint, workspace, pane, source, capability, and UUID. The handler sends a positive acknowledgement only after the metadata save succeeds; rejection or save failure produces a negative acknowledgement that the hook client retries within a bounded attempt budget.
- Before any hook metadata or capability is written, the IPC client verifies that the connected named-pipe server has the same Windows user SID, or that the Unix peer has the same effective uid and an owner-private socket inode. Servers independently reject different-owner Unix peers, including connections queued during the bind-to-permission transition.
- Managed hook configuration and lock files must be direct regular files. Symlinks and Windows reparse points are rejected for both installation and removal, and concurrent editors are serialized with an OS-released advisory lock.
- Disabling automatic launch leaves the capture hooks installed so exact ids remain available to the manual Resume button. The user-facing `Remove recovery hooks` control removes only entries carrying Terminal Manager's unique marker and never removes user-authored hooks.
- Codex may still ask the user to trust newly installed hooks; Terminal Manager does not bypass that trust flow.

### Persistence and crash safety

- `workspaces.json` writes use a same-directory temporary file, flush and sync it, and atomically replace the destination.
- Workspace state and recovery telemetry are created owner-private and tighten legacy Unix permissions to `0600` on their next write.
- A legacy file without restoration fields loads with restoration disabled and no agent metadata.
- Explicit pane, tab, workspace, and kill-all removal prunes the corresponding restart and pending state.
- Stable workspace ids are persisted independently from list position, so deleting one workspace cannot redirect a restored pane into another workspace's daemon key.
- Every keep-running or kill-all close route applies its intended live mutation, persists the resulting layout/recovery state, and exits only after that save succeeds. A failed save leaves the UI open with a retryable error; a failed post-kill save keeps the empty live state so a retry cannot resurrect stale agents.

### Observability

- Restoration state transitions emit bounded JSONL events to `agent-restore-events.jsonl` in the active profile config directory.
- Every event has a stable event name, level, Unix timestamp, and correlation id. Applicable events include provider, workspace id, pane id, source, outcome, and an allowlisted error category.
- Conversation ids, directories, transcript paths, prompts, output, hook bodies, and raw error strings never appear in telemetry.
- The file is size bounded and rotated outside latency-critical render and PTY-write paths.
- Telemetry unit tests write a representative event to a temporary sink and assert its stable schema and redaction of ids and paths. State and notification tests separately exercise the decisions that emit recovery and hook events.

## UI requirements

- The manual button is rendered inside only the pane awaiting a confirmed cold restore.
- Its label names the provider and is keyboard focusable.
- The Sessions settings section explains that enabling automatic restoration immediately installs managed user-level session-id hooks, disabling automatic launch leaves them installed, and the separate removal control deletes only Terminal Manager's entries.
- Failure is visible through an existing toast and remains retryable.

## Project structure

```text
src/agent_restore/
  mod.rs          durable model, validation, spawn plans, state transitions
  discovery.rs    bounded provider transcript metadata lookup
  hooks.rs        consent-gated hook JSON merge/removal
  telemetry.rs    durable redacted JSONL events

src/notifications.rs        hook CLI and local IPC request
src/persist.rs              pane metadata and atomic workspaces writes
src/pty.rs                  safe reconciliation state
src/state.rs                runtime maps, dispatch, lifecycle pruning
src/ui/settings.rs          opt-in toggle and managed-hook removal control
src/ui/terminal_grid.rs     manual resume button
crates/unshit-ptyd/         atomic ensure and duplicate pane-key rejection
```

## Testing strategy

- Unit tests cover id validation, exact resume argv, legacy serde defaults, prompt redaction, metadata round trip, discovery ambiguity, and telemetry redaction.
- Hook config tests use temporary Claude/Codex files and prove idempotent merge, preservation of unrelated entries, managed-only removal, malformed JSON refusal, crash-safe lock recovery, and linked-file rejection.
- Notification and transport tests cover the 64 KiB stdin bound, event/source allowlist, per-PTY capability validation, peer-owner rejection, cold-listener retry, stale-target rejection, and prompt/transcript field exclusion.
- Daemon and client tests cover atomic ensure under concurrency, lost-response retry without executing the fallback, transient cached-attach retention, typed stale-cache `NotFound` reconciliation, and duplicate live pane-key rejection.
- State and UI tests cover opt-in default/persistence, pending and confirming launch phases across UI reattachment, pending button projection, malformed dispatch rejection, failed retry retention, workspace-id stability, lifecycle pruning, and close-save failure vetoes for remembered and dialog policies.
- Focused tests run after each vertical slice, followed by `cargo fmt --check`, `cargo clippy -- -D warnings`, and `cargo test`.

## Success criteria

- Closing and reopening only the UI reattaches to the original PTY with no new agent process.
- Restarting with no daemon and a recorded exact Claude/Codex id produces the manual resume affordance by default.
- With auto-resume enabled, the same cold start launches the exact provider resume argv in the recorded directory.
- A failed or incomplete session-list cache cannot select a resume fallback; only the daemon's atomic `EnsureSession` decision can spawn it.
- Session-id changes are persisted before graceful close is required.
- Telemetry proves which branch ran without exposing conversation or filesystem content.

## Boundaries

- Persisting arbitrary processes or replaying arbitrary shell commands is out of scope.
- Automatically starting Terminal Manager at Windows login is separate and is not registered by this feature. Recovery runs when the user next opens the app.
- Uploading or copying provider transcripts is out of scope.
- Bypassing Claude/Codex permission, authentication, or hook trust prompts is out of scope.

## Provider references

- Claude Code hook and SessionStart payload contract: <https://code.claude.com/docs/en/hooks>
- Claude Code session resume contract: <https://code.claude.com/docs/en/sessions>
- Codex hook and trust contract: <https://learn.chatgpt.com/docs/hooks>
- Codex CLI resume command reference: <https://learn.chatgpt.com/docs/developer-commands?surface=cli>
