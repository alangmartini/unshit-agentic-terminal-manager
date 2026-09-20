### Fixed

- **Manually started agents are filed under `agents`.** Typing `codex`,
  `claude`, `gemini`, `opencode`, `aider` or `copilot` into a plain
  terminal used to leave the pane under `terminals` unless the harness
  happened to set a recognisable window title. The Windows background
  resource monitor now recognises native agent executables and the known
  runtime entrypoints the npm-installed CLIs run through inside each
  session's process tree, moves the pane to `agents` on the next
  successful scan (normally within a second) and clears the tag again
  when the harness exits. Explicit launches and session hooks keep their
  priority, titles remain a fallback for harnesses the scan does not
  recognise, and a scan that cannot read a runtime's command line leaves
  the pane's membership unchanged rather than claiming the agent exited.
  Restored layouts drop persisted process tags until a fresh scan confirms
  them. Each transition lands in `agent-events.jsonl` as `agent.classified`
  / `agent.untagged` with `"source":"process"`.

### Added

- **Custom agent detection rules.** `agent-detection.json` beside
  `workspaces.json` in the profile's config directory lists extra
  harnesses to recognise by executable basename and, optionally, an exact
  argument prefix. The monitor re-reads the file once a second, an invalid
  edit keeps the last valid rules active, and deleting the file removes
  custom detection. Each reload transition lands in `agent-events.jsonl`
  as `agent.rules_loaded` (with the rule count) or `agent.rules_rejected`
  (with the fixed-template reason, never the file's contents), so a rule
  that is not taking effect can be diagnosed from telemetry alone.
  Built-in detection wins over custom
  rules and the outermost detected harness wins when agents launch other
  agents. Rules classify panes only; they add no launch command or
  conversation recovery. `assets/agent-detection.example.json` shows the
  format and the README documents the limits.
