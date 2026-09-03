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
- Flow Explorer call stack view: view and level tabs, expand/collapse, process-coloured tree rows with descriptions, locations and `src` buttons, and a legend of processes and event carriers (`flow.view:`, `flow.level:`, `flow.expand_all`, `flow.collapse_all`, `flow.toggle:<row>`, `flow.src:<id>`).
- Flow Explorer inline source: the `src` button (and the source level) opens a syntax-coloured excerpt of the node's location with line numbers and the located range highlighted; excerpts are read once per node from the flow's repo root (path-contained, 256 KiB cap) and a stale location renders "source not available" instead of failing.
- Flow Explorer keyboard navigation: Up/Down/Home/End/PageUp/PageDown move a row cursor, Right expands or steps into a child, Left collapses or steps out, Enter/Space toggles, `s` opens the source, `e`/`c` expand or collapse all, Ctrl+1/2/3 switch views; clicking a row selects it and the cursor follows collapses and level changes.
- Flow Explorer panes view: Miller columns from the flow overview through each chosen node (name, kind, process, carrier, tags, description, payload, calls / resolves / handled by / handles lists and the source excerpt); older columns collapse to rotated strips, `flow.select:<col>:<id>` and `flow.focus:<col>` drive it, and the shared cursor keys move a column cursor.
