### Added

- **Agent attention notifications.** Settings can now install and remove
  Terminal Manager's Claude Code and Codex lifecycle hooks. Finished turns and
  permission requests produce targeted in-app and macOS/Windows desktop
  notifications, while both the horizontal tab strip and the vertical workspace
  row blink until the originating pane is focused. Harnesses without a common
  hook contract (including OpenRouter-backed agents) can use the existing
  `terminal-manager notify` command shown in the same settings section.
