### Added

- **Flow Explorer: a native pane that shows an agent-authored model of one
  user-facing flow through a codebase, instead of a raw diff.** The agent
  writes a small JSON document (nodes tagged with the process they run in
  and the carrier an event travels over, ordered edges, entry points, and
  per-node diff status for review mode); the app renders it. This slice
  lands the data model: two-phase parsing so a producer's own failure
  reason reaches the user instead of a "missing field" error, validation
  that reports every problem at once, bounded call-stack tree derivation
  (cycles become "shown above" leaves; depth and row caps stop a hostile
  document), and a committed fixture transcribed from the reference video.
  Location paths a Windows agent writes with backslashes are normalized
  rather than rejected.
- A flow opens as its own pane: **Open flow…** in the command palette (or
  `flow.open:<path>` from the startup dispatch hook) reads a flow JSON, shows
  its title, summary and counts, and lives beside terminals and editors in
  the tab strip. Flow panes never enter the PTY paths and are not persisted
  across restarts (the JSON stays on disk under the profile's `flows/`
  directory and can be reopened). Every open and close is recorded in
  `flow-events.jsonl` next to the editor's log, with reasons for failures.
