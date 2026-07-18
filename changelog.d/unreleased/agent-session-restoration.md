### Added

- Restore Claude Code and Codex conversations after the PTY daemon is lost, including a machine restart. When Terminal Manager is next opened, saved agent panes with an exact or unambiguous conversation id show a provider-specific manual Resume chip by default, with an opt-in automatic mode under Settings > Sessions. This feature does not register Windows login startup.
- Capture exact conversation ids through consent-gated, idempotent Claude Code and Codex SessionStart hooks. Enabling automatic recovery installs the managed hooks immediately; disabling it leaves them available for manual recovery, and a separate Remove recovery hooks control removes only Terminal Manager entries.
- Preserve stable numeric workspace ids so deleting one workspace cannot renumber another workspace's daemon session keys.

### Changed

- Reconcile restored panes with a daemon-atomic attach-or-spawn request, preventing duplicate agent launches when clients race, the initial session-list cache is unavailable, or an IPC response is lost.
- Authenticate hook observations with per-PTY capabilities and acknowledge them only after the recovery record has been durably saved, with bounded negative-acknowledgement retries.
- Verify local IPC peer ownership before either side accepts recovery traffic, and refuse linked/reparse-point hook configuration files so another local account or filesystem redirect cannot capture a capability or overwrite an unintended file.
- Write minimal routing and launch metadata through owner-private, synced atomic replacement, inspect only a bounded JSONL prefix for discovery, and emit size-bounded, redacted recovery events. Persistence and telemetry exclude prompts, transcript content, terminal output, hook payloads, conversation ids, paths, and raw errors as applicable.
- Veto application exit when the final keep-running or kill-all recovery state cannot be saved. The updated live state remains open with an actionable error, preventing stale disk metadata from resurrecting an intentionally stopped agent.
