### Added

- **Agents subtab in the workspace sidebar.** Every workspace now shows two
  pane lists, `terminals` and `agents`, each with its own count and fold.
  A pane lands under `agents` when the app launched the agent itself, when
  a provider SessionStart hook reported it, or, with no setup at all, when
  the guest window title identifies a known agent CLI (Claude Code's
  status-glyph titles, `Claude`, `Codex`, `Gemini`, `OpenCode`, `Aider`,
  `Copilot`, `OpenRouter`). Title-based membership clears again when the
  title stops matching, so a shell that ran `claude` returns to
  `terminals` once the agent exits. Agent rows and their tabs in the tab
  strip carry an agent glyph, and membership survives a restart
  (`agent_tag` on the persisted pane; older files still load and are
  re-classified from the saved title).
- **New agent, everywhere.** `Ctrl+Shift+A` (editable as *New agent*
  under Settings › Keybinds), the palette rows *New agent* / *New Claude
  Code agent* / *New Codex agent*, a **New agent ›** flyout on the
  workspace and subtab context menus listing the installed agent CLIs,
  and `terminal-manager agent [claude|codex|gemini|opencode|aider|copilot]
  [--workspace-id N]` from any terminal (defaults to the calling
  terminal's workspace and brings the window forward) all open a tab
  running the agent in the workspace directory (honouring the worktree
  tabs toggle). Claude launches with a pre-generated `--session-id` so
  the existing crash-recovery record is exact; Codex runs interactively.
- Right-clicking the `agents` subtab offers **Kill all agents**, which
  kills only the workspace's agent panes after a confirmation and leaves
  plain terminals untouched; the `terminals` subtab menu carries the
  shell flyout and **Kill all terminals**.
- Structured `agent-events.jsonl` under the profile directory records
  classification transitions, launches, kills and CLI requests
  (`agent.classified`, `agent.untagged`, `agent.launch`,
  `agent.launch_failed`, `agent.kill_all`, `agent.cli`) with profile,
  source and reason fields and never the guest title text.
