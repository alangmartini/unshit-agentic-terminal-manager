# Spec: Agents subtab

Issue: https://github.com/alangmartini/unshit-agentic-terminal-manager/issues/186
Status: shipped 2026-09-05 on `worktree-feat-agents-tab`.

## Objective

Panes that run an agent CLI (Claude Code, Codex, Gemini CLI, OpenCode, Aider, Copilot CLI) are filed under a second sidebar subtab, `agents`, instead of being mixed into `terminals`. The app can start a new agent from a hotkey, the palette, the workspace and subtab context menus and a CLI command, and can kill every agent in a workspace without touching plain shells.

## User stories

- As a user running several agents next to a few shells, I want the sidebar to separate them so I can see at a glance how many agents each workspace is running and jump to one.
- As a user, I want `Ctrl+Shift+A` to start my default agent in the active workspace without opening a shell and typing the command.
- As a user, I want to right-click a workspace or its `agents` subtab and pick which installed agent to start.
- As a script or an agent inside a terminal, I want `terminal-manager agent [profile]` to open a new agent tab in my workspace.
- As a user, I want "Kill all agents" for a workspace to leave my shells alone.
- As a user who only ever types `claude` into a shell, I want that pane to land under `agents` anyway, without installing hooks.

## Acceptance criteria

### F1. Classification

- A pane belongs to `agents` when it carries an `AgentTag { profile, source }`. Sources, highest priority first:
  1. `launched`: the app spawned the agent (hotkey, palette, context menu, CLI, Quick Prompt, Flow Explorer producer).
  2. `hook`: a provider SessionStart hook or a managed launch left an `agent_restarts` record for the pane (the existing crash-recovery record).
  3. `title`: the guest window title (OSC 0/2) matches a profile. Claude Code is recognised by its status-glyph prefix (`✳ ✶ ✻ ✽ ✢ · ◐ ◓ ◑ ◒`) or the word `claude`; the others by a whole-word needle (`codex`, `gemini`, `opencode`, `aider`, `copilot`, `openrouter`) after path tokens are stripped, so `C:\Users\me\project` never matches and `claude.exe` does.
- `agent_restarts` outranks `pane_agents`: a pane with a recovery record shows the record's profile.
- Only `title` tags clear. When a title stops matching, the tag is removed and the pane returns to `terminals`; `launched` and `hook` tags persist until the pane closes.
- Classification runs on every title update, before the custom-title early return, so a pane the user renamed with F2 still moves to `agents`. Unknown pane ids never get a tag.
- Classification never changes the pane label. `mutate_apply_osc_title` keeps its return contract ("label changed").

### F2. Sidebar

- Each workspace shows two subtabs in the tree: `terminals` (├) and `agents` (└), each with its own count and fold state (`terminals_expanded`, `agents_expanded`). The workspace meta count is the sum.
- The subtab that holds the active pane is highlighted. Agent rows carry an amber agent glyph; so do their tabs in the tab strip.
- Right-clicking a subtab opens a scoped context menu: header `<workspace> · <kind>`, the matching **New … ›** flyout (shells for `terminals`, installed agents for `agents`), and a danger row **Kill all terminals** / **Kill all agents**.
- The workspace context menu gains **New agent ›** after **New terminal ›**.

### F3. Starting an agent

- `Ctrl+Shift+A` (`KeybindAction::NewAgent`, editable in Settings › Keybinds) dispatches `agent.new`, which resolves `default_profile()`: the first profile whose program is installed, in table order (Claude Code, Codex, Gemini CLI, OpenCode, Aider, Copilot CLI), falling back to Claude Code when none is found.
- Palette rows: **New agent**, **New Claude Code agent**, **New Codex agent** (Session group).
- Context menus list `menu_profiles()`: the profiles whose program is on `PATH` (with `PATHEXT` on Windows), or every launchable profile when none is installed so the menu is never empty.
- CLI: `terminal-manager agent [claude|codex|gemini|opencode|aider|copilot] [--workspace-id N] [--socket PATH]`. The profile accepts the aliases `claude-code`, `claudecode`, `cc`, `gemini-cli`, `copilot-cli`. The workspace defaults to `TM_WORKSPACE_ID` (set in every pane the app spawns). The request travels over the existing notification IPC as `NotificationIpcRequest::NewAgent`, switches the window to that workspace, opens the tab and brings the window forward. Unknown profiles fail before the socket is touched with the list of known agents.
- The launch runs in the workspace directory, or in a fresh worktree when the workspace has worktree tabs enabled, exactly like a terminal tab.
- Claude launches with `--session-id <uuid>` and a managed `AgentRestart` record so crash recovery is exact. Codex launches interactively with no arguments. The other profiles run their bare program.
- An unknown id on `agent.new:<id>` shows a toast naming the known agents and records `agent.launch_failed`.

### F4. Killing agents

- **Kill all agents** on the `agents` subtab (or `workspace.request_kill_agents:<idx>`) opens `ConfirmDialog::KillAgents { workspace_idx, name, count }`. Confirming closes only the workspace's agent panes, pruning the layout like a normal close; plain terminals survive. With zero agents the command shows a toast instead of a dialog.

### F5. Persistence

- `PersistedPane.agent_tag` stores the tag. On restore the tag is rehydrated; older files without the field are classified from the saved title (only when the pane has no recovery record and no custom title, so a rename is never mistaken for an agent).
- The persisted label for a launched agent is the profile label, not the live title, so a restored pane reads "Claude Code" rather than a transient status line.

### F6. Telemetry

- `agent-events.jsonl` under the profile directory (`crate::agents::telemetry`, rotating JSONL) records, with `correlation_id = restore_correlation_id`, `workspace_id`, `pane_id`, `profile`, `source`, `reason`, `outcome`, `error_kind`:
  - `agent.classified` / `agent.untagged` on title transitions only (never per title update);
  - `agent.launch` / `agent.launch_failed` for every launch surface (`source` = `hotkey`, `palette`, `ctx_menu`, `cli`, …);
  - `agent.kill_all` with the count;
  - `agent.cli` for IPC requests with the outcome.
- Guest titles are never written to the log.

## Commands

| Command | Effect |
|---|---|
| `agent.new` | Start the default agent in the active workspace. |
| `agent.new:<id>` | Start a specific profile. |
| `workspace.new_agent:<idx>[:<id>]` | Switch to workspace `idx` and start an agent (context menus). |
| `workspace.request_kill_agents:<idx>` | Confirm, then close the workspace's agent panes. |
| `ctx_menu.open_subtab:<idx>:<terminals\|agents>:<x>:<y>` | Open the subtab context menu. |

## Project structure

- `src/agents/mod.rs`: profiles, `AgentTag`, `classify_title`, installed-profile scan.
- `src/agents/telemetry.rs`: `AgentEventRecord` and the JSONL sink.
- `src/state.rs`: `pane_agents`, `classify_pane_title`, `mutate_add_agent_tab`, `agent_launch_plan`, `mutate_kill_workspace_agents`, the dispatch arms, `default_subtabs`, snapshot partitioning.
- `src/persist.rs`: `agent_tag` on `PersistedPane`, `persisted_agent_label`.
- `src/ui/sidebar.rs`, `src/ui/tabbar.rs`, `src/ui/confirm_dialog.rs`, `src/ui/icons.rs`, `assets/styles.css`: rendering.
- `src/keybinds/mod.rs`, `src/command_palette.rs`, `src/notifications.rs`: launch surfaces.
- `scripts/agents-tab-shot.ps1`: isolated end-to-end capture (title-only classification plus the subtab menu) and the telemetry check.

## Testing strategy

- Unit tests in `src/agents` for every classification rule (glyph prefix, needle, path tokens, non-matching titles, aliases).
- `agents_tab_tests` modules in `state.rs`, `persist.rs`, `sidebar.rs`, `tabbar.rs`, `notifications.rs`, `command_palette.rs`, `keybinds` cover tag priority, title clearing, snapshot partitioning, the launch plan, the kill dialog scope, persistence round trips and legacy classification, menu contents and the CLI parser.
- PTY spawn fails in tests (`NotConnected`), so launch tests assert on `agent_launch_plan` and on the tag rather than on a spawned pane.
- Visual check: `pwsh scripts/agents-tab-shot.ps1` renders a pane classified purely from the title, with the `agents` subtab menu open, and prints the `agent.*` telemetry lines from the isolated profile.

## Boundaries

- No process-tree inspection: a pane whose agent suppresses or rewrites its title stays under `terminals` unless the app launched it or a hook reported it (see BACKLOG).
- No per-agent configuration UI; profiles are a static table.
- OpenRouter is title-only (no CLI to launch).

## Open questions

- The Codex, Gemini, OpenCode, Aider and Copilot needles come from the binaries' names; their actual Windows titles are unverified and may need glyph or prefix rules like Claude's.
- Whether "open router" in the request meant a specific client that should become launchable.

## Decisions log

- Title classification is a side effect of the existing OSC title path rather than a separate poll, so it costs nothing on the hot path and needs no new IPC.
- `agent_restarts` outranks `pane_agents` because the recovery record is authoritative about which provider owns the pane.
- Managed Claude launches pin `--session-id` so the recovery record is exact from the first frame instead of waiting for a hook.
- The subtab menu reuses the workspace menu's flyout builders, so the shell list and the agent list stay in one place.
