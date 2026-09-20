# Flow skills in ordinary agent conversations

## Outcome

Settings > Agent skills installs the bundled `flow-explorer` skill for local
Codex, Claude Code, Cursor, and GitHub Copilot sessions across projects. A user
can ask an existing conversation to visualize an explanation or concept, as
well as a code flow or change. The skill produces the existing Flow JSON format.

## Acceptance criteria

- Show each personal installation path and its actual filesystem status.
  Install, update, and remove only the files owned by this feature. Preserve
  conflicting/customized skills and unrelated files. No install on startup.
- Cache status for rendering; refresh when entering the settings page or after
  an action. Report filesystem errors in the UI; do not persist a second copy
  of installation state in app configuration.
- Keep one embedded skill body for both app launches and personal installs.
  Conceptual explanations use conversation context, logical operations/events/
  states and lanes, with no invented source locations or Git metadata.
- Standalone invocation has a default output directory and unique filenames.
  The installed skill documents `terminal-manager flow open <absolute-path>`;
  local IPC opens valid documents in the caller's workspace (or the active
  workspace outside Unshit). Reject invalid documents/targets before adding a
  tab, and return a failing CLI exit code. An unavailable app leaves the file
  intact and the skill tells the user how to open it manually.
- Existing palette launches and schema version 1 remain compatible.

## Agent locations

Verified against provider documentation on 2026-09-19:

| Agent | Personal skill root | Reference |
| --- | --- | --- |
| Codex | `~/.agents/skills` | https://learn.chatgpt.com/docs/build-skills |
| Claude Code | `~/.claude/skills` | https://code.claude.com/docs/en/skills |
| Cursor | `~/.cursor/skills` | https://cursor.com/docs/skills |
| GitHub Copilot | `~/.copilot/skills` | https://docs.github.com/en/copilot/how-tos/copilot-on-github/customize-copilot/customize-cloud-agent/add-skills |

Some clients also discover other clients' skill directories. Settings manages
the listed destination, not a client's enable/disable preferences. These are
local installs; remote/cloud sessions require their own distribution.

## Verification

Temporary-home tests cover installation, updates, conflicts, removal, and
errors. CLI/IPC tests cover routing and invalid input. A conceptual fixture
must parse and render in all three views without source snippets. Run format,
Clippy, the app tests, and an isolated desktop smoke check of the settings and
CLI handoff. No tests write to real agent skill folders.

The desktop smoke script can exercise the handoff without a second UI:

```powershell
./scripts/flow-explorer-shot.ps1 -ViaCli `
  -Fixture tests/fixtures/flow-explorer/cache-concept.json `
  -Out target/flow-cli.png
```

The conceptual fixture also covers call-stack row layout and branch labels in
the graph. The scrolling call-stack container explicitly lays out its children
in a vertical flex column so their widths remain visible after UI rebuilds.
