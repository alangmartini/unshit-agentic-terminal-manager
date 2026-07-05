### Added

- Instance profiles isolate parallel app instances from each other. Every
  OS-shared resource — the `unshit-ptyd` daemon pipe, the notification pipe,
  and the config dir (`workspaces.json`, `quick_prompt.json`,
  `keybindings.json`, Quick Prompt worktrees) — is now namespaced by a profile:
  - The **installed app** keeps the unsuffixed defaults (`com.godly.terminal`,
    `\\.\pipe\unshit-ptyd-<user>`), so nothing changes for daily use.
  - **Repo builds** (`cargo run`, debug or release, any `target*` dir)
    automatically run in the `dev` profile with their own daemon, sessions,
    and config — dogfooding a work-in-progress build can no longer attach to
    the installed app's sessions or overwrite its workspace layout.
  - `TM_PROFILE=<name>` selects an explicit profile (`TM_PROFILE=default`
    forces the installed-app namespace); `TM_CONFIG_DIR` additionally
    redirects the config dir, which tests use to stay fully ephemeral.
  The window title shows the active profile (e.g. `terminal manager [dev]`).

### Fixed

- Test harnesses and helper scripts can no longer disturb a running session:
  - `cargo xtask desktop-regression` launches every app session in a unique
    throwaway profile (own daemon pipe, temp config dir) and its pre-build /
    post-test process cleanup now matches executables by *path* (repo
    `target\debug` builds only) instead of killing every `terminal-manager.exe`
    / `unshit-ptyd.exe` by name — the installed app and its daemon are never
    collateral damage.
  - `scripts/kill-all.ps1` is repo-scoped by default (only kills processes
    running from this repository's build dirs) and requires `-All` to touch
    anything else.
  - Screenshot helpers (`palette-shot.ps1`, `software-renderer-shot.ps1`) run
    the app in an ephemeral profile via `scripts/lib/tm-isolation.ps1` and shut
    their daemon down afterwards.
