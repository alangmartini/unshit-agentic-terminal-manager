# Implementation Plan: Agent Conversation Restoration

## Dependency graph

```text
durable model + redacted telemetry
    -> atomic pane persistence
    -> Quick Prompt identity + hook IPC write-through
    -> shared cold-spawn resolver

daemon-atomic ensure + duplicate-key guard
    -> safe active/background/deferred startup wiring
    -> pending manual affordance or automatic resume

consent-gated hook merge
    -> Sessions setting
    -> exact ids for manually launched Claude/Codex sessions
```

## Task 1: Define the restart model and resume commands

Files: `src/agent_restore/mod.rs`, `src/quick_prompt/spawn.rs`, `src/main.rs`

Acceptance:

- Provider, durable record, candidate confidence, pending presentation, and spawn plan are typed.
- Conversation ids accept UUID syntax only and are never embedded in a shell string.
- Claude and Codex exact resume argv match the installed CLIs.
- Claude Quick Prompt can start with a generated UUID.

Verification:

- `cargo test -p terminal-manager agent_restore`
- `cargo test -p terminal-manager quick_prompt::spawn`

Dependencies: none. Risk: CLI contract drift, mitigated by source-grounded argv tests and documentation.

## Task 2: Make workspace persistence power-loss safe

Files: `src/persist.rs`, `src/state.rs`

Acceptance:

- Optional restart records round trip and legacy JSON defaults to none.
- Atomic replace preserves the prior valid destination until the new body is synced.
- Pane/tab/workspace/kill-all removal prunes runtime records.
- Workspace numeric ids persist independently from list position; removing one workspace does not renumber survivors or redirect daemon pane keys.
- Quick Prompt records metadata before its immediate workspace save.
- Serialized JSON contains no prompt or transcript content.

Verification:

- `cargo test -p terminal-manager persist`
- focused lifecycle state tests

Dependencies: Task 1. Risk: persistence failure; close paths must remain open with actionable telemetry until the resulting state is saved.

## Task 3: Harden daemon reconciliation

Files: `src/pty.rs`, `crates/unshit-ptyd/src/session/registry.rs`, daemon tests

Acceptance:

- Initial list failure does not classify panes as absent or choose a spawn fallback; the list is only a cache optimization.
- A transient cached attach failure remains retryable and does not spawn, while a typed stale-cache `NotFound` falls through to the authoritative atomic ensure.
- A daemon `EnsureSession` request performs lookup-or-spawn under one registry lock and is idempotent after a lost response.
- A live duplicate `(workspace_id, pane_id)` is rejected by the ordinary spawn path too.

Verification:

- focused `DaemonPty` attach-or-spawn tests
- `cargo test -p unshit-ptyd registry`
- existing detach/reattach integration tests

Dependencies: none. Risk: startup availability; only `EnsureSession` may decide that a pane is absent and execute the supplied fallback.

## Task 4: Capture exact ids through the local IPC

Files: `src/notifications.rs`, `src/agent_restore/hooks.rs`, `src/persist.rs`

Acceptance:

- `session-hook claude|codex` is parsed separately from notification text and remains silent for provider hook compatibility.
- Hook stdin is size bounded and only `session_id`, `cwd`, and the required `SessionStart` event/source fields are extracted.
- Workspace and pane ids come only from the daemon-injected environment; tests provide an isolated environment map.
- Each daemon-owned PTY receives an unguessable hook capability. `AgentSessionObserved` requests validate the endpoint, OS peer owner, target, source, capability, and UUID, then acknowledge only after the updated metadata has been durably saved; negative acknowledgements are retried within a bounded attempt budget.
- Hook config merge/removal is atomic, advisory-locked, rejects symlinks/reparse points, preserves unrelated settings, and only runs after explicit opt-in.

Verification:

- `cargo test -p terminal-manager notifications`
- `cargo test -p terminal-manager agent_restore::hooks`

Dependencies: Tasks 1 and 2. Risk: user-level config mutation; production paths are gated and tests use temporary files only.

## Task 5: Resolve fallback candidates and emit durable telemetry

Files: `src/agent_restore/discovery.rs`, `src/agent_restore/telemetry.rs`

Acceptance:

- Claude and Codex metadata probes read at most a 128 KiB, 64-record JSONL prefix, extract only id and cwd, and match normalized cwd and recency without retaining other content.
- Zero or multiple equally plausible candidates produce no automatic candidate.
- JSONL events are stable, correlated, size bounded, and redact ids/paths/content/errors.
- Test-triggered events are queryable from a temporary sink.

Verification:

- `cargo test -p terminal-manager agent_restore::discovery`
- `cargo test -p terminal-manager agent_restore::telemetry`

Dependencies: Task 1. Risk: provider file format changes; exact hook ids remain primary and discovery fails closed.

## Task 6: Wire all cold-start routes and manual/automatic behavior

Files: `src/main.rs`, `src/bridge.rs`, `src/state.rs`

Acceptance:

- Active startup, background startup, and deferred spawn use one resolver.
- Warm attach ignores resume fallbacks and clears stale pending UI.
- Confirmed cold miss creates a pending button when auto is off and launches structured resume argv when on.
- Manual resume replaces the temporary daemon session and launches only structured provider argv through atomic reconciliation. The button is hidden during confirmation; immediate or pre-confirmation exit returns the candidate to a retryable state.
- One failed pane does not stop other targets.

Verification:

- spawn-plan unit tests for each reconciliation/outcome branch
- focused state dispatch tests
- startup/reattach integration coverage

Dependencies: Tasks 1 through 5. Risk: duplicated startup code; shared resolver and outcome handler are mandatory.

## Task 7: Add the opt-in setting and resume button

Files: `src/ui/settings.rs`, `src/ui/terminal_grid.rs`, `assets/styles.css`, `src/state.rs`

Acceptance:

- Sessions shows a default-off, keyboard-operable automatic restore toggle with hook-consent disclosure.
- A pending pane shows the provider-specific resume button and no sensitive metadata.
- Toggle install failure leaves the setting off and shows a retryable toast.
- Enabling automatic launch immediately installs the managed capture hooks. Disabling it keeps those hooks for manual recovery; a user-facing `Remove recovery hooks` control removes only Terminal Manager managed entries.

Verification:

- focused settings and terminal-grid UI tests
- app launch/manual visual smoke when practical

Dependencies: Tasks 4 and 6. Risk: UI overlap with existing work; additions remain isolated near current pane and Sessions styles.

## Task 8: Review, changelog, and release gates

Files: `changelog.d/unreleased/agent-session-restoration.md`, documentation as needed

Acceptance:

- Security review confirms no prompt, transcript, id, cwd, hook payload, or raw error leaks to telemetry.
- Code review confirms every resume fallback can execute only inside daemon-atomic `EnsureSession`, and every metadata mutation persists immediately.
- Existing user-owned dirty hunks remain unmodified and unstaged.
- Related implementation is committed atomically with conventional messages and no AI attribution.

Verification:

- `cargo fmt --check`
- `cargo clippy -- -D warnings`
- `cargo test`
- `git diff --check`
- `git status --short`

Dependencies: all prior tasks.
