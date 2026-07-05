# Dogfooding: run the installed app daily while developing it

The app supports **instance profiles** so you can use a stable installed copy
as your daily terminal while simultaneously building, testing, and screenshotting
work-in-progress builds of the same app — without the two ever sharing a daemon,
a session, or a config file.

## The three tiers

| Tier | Who runs it | Profile | Daemon pipe | Config dir |
|------|-------------|---------|-------------|------------|
| Installed app | you, daily | *(default)* | `\\.\pipe\unshit-ptyd-<user>` | `%APPDATA%\com.godly.terminal` |
| Repo build | `cargo run` (debug **or** release) | `dev` (automatic) | `...-dev` | `%APPDATA%\com.godly.terminal.dev` |
| Tests / scripts | xtask desktop-regression, shot scripts | ephemeral per run | unique per run | temp dir, deleted after |

Profile resolution, in order:

1. `TM_PROFILE=<name>` wins (`TM_PROFILE=default` forces the installed-app
   namespace even for a repo build).
2. Otherwise any binary built with debug assertions, or whose exe lives under a
   cargo `target*` directory, runs as `dev`.
3. Otherwise (installed copy) the default profile.

`TM_PTYD_SOCKET` still overrides the daemon pipe directly, and `TM_CONFIG_DIR`
redirects the config dir (tests use both to stay fully ephemeral). The window
title shows the active profile — `terminal manager [dev]` — so you always know
which instance you're looking at.

## Daily use

Install once:

```powershell
cargo build --release
& "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe" packaging\terminal-manager.iss
dist\terminal-manager-<version>-setup.exe
```

Launch "Terminal Manager" from the Start menu. Its sessions live on the default
pipe and survive UI restarts via the daemon, exactly like before.

> **One-time migration:** if you previously ran `cargo run --release` as your
> daily terminal, the old daemon lives in `target\` and holds your current
> sessions. Close it deliberately once (`scripts\kill-all.ps1` — repo-scoped,
> or just quit with kill-all-on-close) and switch to the installed app. Live
> shells cannot be transplanted between daemons.

## Developing while the installed app runs

- `cargo run` / `cargo run --release` → `dev` profile. Own daemon, own
  persisted layout, `[dev]` in the title. Rebuilding never bumps into the
  installed app, and closing the dev UI leaves the *dev* daemon (not your real
  one) holding dev sessions.
- `cargo xtask desktop-regression ...` → each app session gets a unique
  `reg<pid>x<n>` profile, a temp config dir (deleted afterwards), and its
  daemon is shut down after the suite. Its pre-build "stop processes that lock
  binaries" step only ever kills processes running from `target\debug`.
- `scripts\palette-shot.ps1`, `scripts\software-renderer-shot.ps1` → ephemeral
  `shot<pid>x<n>` profile via `scripts\lib\tm-isolation.ps1`; the daemon is
  shut down and the temp config dir removed when the shot completes.
- Writing a new script that launches the app? Dot-source
  `scripts\lib\tm-isolation.ps1` and wrap the launch in
  `Enter-TmIsolation` / `Exit-TmIsolation`.

## Cleaning up dev/test leftovers

```powershell
scripts\kill-all.ps1        # repo-scoped: kills only processes running from this repo
scripts\kill-all.ps1 -All   # also kills the installed app + its daemon
```

The default can never touch the installed app, so it's always safe to run
between dev iterations.
