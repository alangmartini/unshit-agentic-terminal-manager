# Spec: Self-update

Status: built 2026-09-07 on `worktree-soft-petting-wave`. The in-app half
(check, prompt, settings, download, verify, hand-off) is exercised end to end
against a fake feed by `scripts/update-shot.ps1`, and the installer half (wait
for the parent and for both executables, install silently, relaunch) by
`scripts/update-rehearsal.ps1`, which installs and updates a separately
identified "TM Rehearsal" copy built from the real `.iss` and binaries. The
manual check was also run against the live GitHub feed. The first update of a
user's real install happens when the release *after* the one that carries this
code is installed through the app.

## Objective

Installing a new build no longer means opening the Releases page, downloading
the setup and running it by hand. The app checks GitHub Releases shortly after
startup, prompts once per new version, and can download and install the
release installer in place from a button in Settings. The update restarts the
app (daemon included) and brings the workspace layout back with fresh shells.

## User stories

- As a user who runs the installed app every day, I want to be told once when a
  new version exists and be able to install it from that dialog, or say "later"
  and not be nagged again for that version.
- As a user, I want **Settings › Updates** to show my version, let me check on
  demand, install when something newer exists, and switch the startup check off.
- As a user with a dozen agent panes open, I want the update to tell me it will
  close every terminal session before it does so, and never to install on its
  own.
- As a user who built the app from source, I want the check to still work but
  the install button to send me to the release page instead of trying to
  replace binaries the installer does not own.
- As a developer, I want a dev or test instance never to poll GitHub, and a way
  to point the updater at a fake feed on disk for screenshots and e2e runs.

## Acceptance criteria

### F1. Feed and version check

- The feed is `https://api.github.com/repos/alangmartini/unshit-agentic-terminal-manager/releases/latest`
  (`updater::DEFAULT_FEED_URL`), fetched with `Accept: application/vnd.github+json`
  and a `terminal-manager/<version>` user agent, body capped at 2 MiB, over the
  platform TLS stack (schannel), 15 s connect / 30 s header timeout. A leading
  UTF-8 BOM is tolerated.
- `tag_name` (`v0.5.0` or `0.5.0`) parses as a semver `Version`; the release is
  *newer* when it compares greater than `CARGO_PKG_VERSION`. Pre-release tags
  compare per semver.
- The installer asset is `terminal-manager-<version>-setup.exe` when present,
  else the first `terminal-manager-*-setup.exe` whose name does not contain
  `non-gpu`. Its `size` and `digest` (`sha256:<hex>`) are kept for
  verification. A release without such an asset is still reported as available
  but cannot be installed (`no_installer_asset`).
- `TM_UPDATE_FEED_URL` replaces the feed URL. Only then are `file://` URLs
  accepted, for the feed and for the asset; a production feed can never redirect
  a download to a local path.
- HTTP 403/429 surface as "GitHub rate limit reached"; any failure sets
  `UpdatePhase::Failed` with a one-line error and is recorded, never toasted at
  startup.

### F2. Startup check and prompt

- After `TM_UPDATE_STARTUP_DELAY_MS` (default 8000) a worker thread runs one
  check per process unless: the `check-updates-on-startup` toggle is off
  (`disabled`); the instance runs under a named/dev profile and the feed is not
  overridden (`dev_profile`); or a check already ran (`already_checked`). Skips
  are recorded as `update.startup_check_skipped` with the reason.
- A newer release opens `ConfirmDialog::UpdateAvailable` exactly once per
  version: `update_prompted_version` is persisted in `workspaces.json` and
  compared before showing. The prompt never replaces a dialog that is already
  open (`dialog_open`); the manual check in Settings ignores the once-per-version
  rule.
- The dialog says which version is ready, which one is running, and that
  installing closes every terminal session and restarts the app. Buttons:
  **Later** (`update.later`, records `update.prompt_dismissed`), **What's new**
  (opens `html_url` in the browser), **Install and restart** (`update.install`).
  While a download is running the dialog shows the progress bar and a single
  **Hide** button. An unmanaged copy gets **Open release page** instead of
  install. `ConfirmDialog::UpdateAvailable` is excluded from the generic
  `dialog.confirm` so no keyboard shortcut can start an install by accident.

### F3. Settings › Updates

- New section between Notifications and Danger Zone (`SettingsSection::Updates`,
  rail icon `icon_download`). Rows:
  - status: `Terminal Manager v<current>` with `UpdateState::status_line()`
    ("Checking…", "You're up to date (checked …)", "Version X is available.
    You're on vY.", the failure text) and **check for updates** (`update.check`,
    disabled/`busy` while a check or install runs);
  - release (only when a newer release is known): `v<version> · <title>`, the
    description of what installing does, **what's new** and either
    **install and restart** / **downloading…** / **verifying…** /
    **installing…** / **try again** (`update.install`) or **open release page**
    for an unmanaged copy. Text stacks above the controls
    (`update-release-row`) so two wide buttons never squeeze the description;
  - progress (only while downloading): bar plus `x MB of y MB (z %)`;
  - **Check for updates at startup** toggle (`update.startup_check.toggle`,
    persisted as `check_updates_on_startup`; absent in old files means on).
- `settings.section:updates` opens the section from the startup dispatch hook.

### F4. Install hand-off

`update.install` runs `begin_install`:

1. Busy → ignored. No newer release known → runs a check with
   `CheckSource::Install`, which chains into the install when one is found.
2. Unmanaged copy (`install_scope` is `None`) → opens the release page and
   records `update.install_redirected`.
3. Otherwise the phase becomes `Downloading`, `update.download_started` is
   recorded and a worker streams the asset to
   `<cache_dir>\updates\<asset>.partial` (`%LOCALAPPDATA%\com.godly.terminal[.<tag>]\updates`),
   hashing as it goes. Size must match; the digest must match when the feed
   has one (`digest_verified`, else `size_only`); the file is then renamed to
   its final name. Progress reaches the UI at most every 256 KB or 100 ms.
4. `finish_install` (on the UI thread, phase `Installing`): persist the layout
   (`update.layout_persisted`; failure aborts with `workspace_write`), launch
   the installer detached with `installer_args`, record
   `update.install_launched` with the child pid and scope, force-stop the daemon
   (`update.daemon_shutdown`; a failure is logged and does not abort), record
   `update.exiting`, and exit the process. If the launch fails the app stays up
   in `Failed` with the error and nothing was stopped.

Installer arguments:

```text
/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /NOCANCEL /NORESTARTAPPLICATIONS
/CURRENTUSER | /ALLUSERS          (from the detected install scope)
/SELFUPDATE=1
/PARENTPID=<app pid>
/RELAUNCH=<path of the running terminal-manager.exe>
/LOG=<installer path>.log
```

Installer side (`packaging/terminal-manager.iss`, `-non-gpu.iss`, `[Code]`):

- `PrepareToInstall` waits up to 60 s for `/PARENTPID` to exit
  (`OpenProcess(SYNCHRONIZE)` + `WaitForSingleObject`), then up to 30 s for
  `unshit-ptyd.exe` and `terminal-manager.exe` to open for exclusive write
  (`CreateFileW(GENERIC_WRITE, share 0)`), and aborts with a message if either
  does not happen. The daemon acknowledges `Shutdown` *before* its process is
  gone, so the file check, not the pid, is what proves the daemon binary can
  be replaced; a silent install would otherwise hit "file in use" and abort.
- `DeinitializeSetup` relaunches `/RELAUNCH` (or `{app}\terminal-manager.exe`)
  as the original user with `ewNoWait` whenever `/SELFUPDATE=1` and the parent
  is gone, on success and on failure alike, so a failed update still brings the
  old app back.
- `PrivilegesRequiredOverridesAllowed=dialog commandline` lets the app pass the
  scope switch; `/ALLUSERS` triggers a UAC prompt.

Install scope detection (`updater::install`): the exe's directory is compared to
`InstallLocation` under
`HKCU|HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall\{B3E1B6B2-7C44-4E2E-9C1A-0A1D2E3F4A5B}_is1`.
`TM_UPDATE_INSTALL_SCOPE=user|machine|none` overrides it (e2e runs use `user`).

### F5. Persistence

- `PersistedState.check_updates_on_startup: Option<bool>` (`None` = on) and
  `PersistedState.update_prompted_version: Option<String>`, both
  `skip_serializing_if = "Option::is_none"`, so old files load unchanged.
- `ToggleKey::CheckUpdatesOnStartup` mirrors the first field in `AppState`.

### F6. Telemetry

`<config_dir>\update-events.jsonl` (rotating JSONL, `updater::telemetry`),
one line per event with `timestamp_unix_ms`, `event`, `level`,
`correlation_id` (= `restore_correlation_id`), `current_version` and, when
relevant, `source` (`startup|manual|install`), `latest_version`, `outcome`,
`reason`, `error_kind` (`transport|parse|bad_tag|http|network|io|size_mismatch|digest_mismatch|too_large|...`),
`http_status`, `bytes`, `total_bytes`, `elapsed_ms`, `scope`, `pid`,
`feed_overridden`. Events:

| Event | When |
|---|---|
| `update.init` | startup; `outcome` = `installed_copy`/`unmanaged_copy`, `scope` |
| `update.startup_check_skipped` | with `reason` = `disabled`/`dev_profile`/`already_checked` |
| `update.check_started` / `update.check_completed` / `update.check_failed` | per check; completed carries `outcome` = `available`/`up_to_date` |
| `update.prompt_shown` / `update.prompt_skipped` / `update.prompt_dismissed` | startup prompt life cycle; skipped carries `reason` = `already_prompted`/`dialog_open` |
| `update.startup_check_toggled` | the settings switch, `outcome` = `on`/`off` |
| `update.release_page_opened` / `update.release_page_failed` | What's new / open release page |
| `update.install_redirected` | install requested on an unmanaged copy |
| `update.download_started` / `update.download_completed` / `update.download_failed` | completed carries `outcome` = `digest_verified`/`size_only`, `bytes`, `elapsed_ms` |
| `update.stale_downloads_removed` | startup sweep of old `.partial`/installer files (the relaunched app deletes the installer it was just updated by); `bytes` = bytes freed, `total_bytes` = number of files |
| `update.layout_persisted` / `update.install_launched` / `update.install_failed` | the hand-off; launched carries the installer `pid` and `scope` |
| `update.daemon_shutdown` / `update.exiting` | last two lines before the process exits |
| `update.worker_spawn_failed` | a check or download thread could not start |

Error text is never used as a label; URLs and paths are not logged.

## Commands

| Command | Effect |
|---|---|
| `update.check` | Manual check (source `manual`). |
| `update.install` | Download, verify and hand off; runs a check first when nothing newer is known. |
| `update.later` | Close the prompt for this version. |
| `update.open_release_page` | Open the latest release in the browser. |
| `update.startup_check.toggle` | Flip and persist the startup check. |
| `update.show_dialog` | Re-open the update dialog (palette / tests). |
| `settings.section:updates` | Open Settings › Updates. |

## Environment

| Variable | Purpose |
|---|---|
| `TM_UPDATE_FEED_URL` | Replace the GitHub feed; enables `file://` for feed and asset. Also lifts the dev-profile startup skip. |
| `TM_UPDATE_STARTUP_DELAY_MS` | Delay before the startup check (default 8000). |
| `TM_UPDATE_INSTALL_SCOPE` | `user`, `machine` or `none`: override registry-based scope detection. |

## Project structure

- `src/updater/mod.rs`: `UpdateState`/`UpdatePhase`, check/install state
  machine, startup worker, hand-off (`finish_install_with` is injectable for
  tests), `updates_dir`.
- `src/updater/feed.rs`: GitHub JSON model, asset selection, digest parsing,
  streaming download with SHA-256.
- `src/updater/transport.rs`: `ureq` agent with platform TLS, `file://` opt-in.
- `src/updater/install.rs`: scope detection (registry), installer argument
  list, detached launch.
- `src/updater/telemetry.rs`, `src/updater/version.rs`.
- `src/profile.rs`: `cache_dir()` (local, non-roaming) for downloads.
- `src/pty.rs`: `DaemonPty::shutdown_daemon_blocking` over the existing
  `ShutdownDaemon` command.
- `src/state.rs`: `ConfirmDialog::UpdateAvailable`, `SettingsSection::Updates`,
  `ToggleKey::CheckUpdatesOnStartup`, dispatch arms.
- `src/persist.rs`: the two new optional fields.
- `src/ui/settings.rs` (`build_updates_section`), `src/ui/confirm_dialog.rs`
  (`build_update_card`), `src/ui/icons.rs`, `assets/styles.css`.
- `packaging/terminal-manager.iss`, `packaging/terminal-manager-non-gpu.iss`:
  `[Code]` hand-off.
- `scripts/update-shot.ps1`: isolated e2e (`dialog`, `settings`, `install`
  modes, `-Unmanaged`).

## Testing strategy

- Unit tests in `src/updater`: tag parsing, asset selection, digest parsing,
  BOM tolerance, a local TCP HTTP server for `fetch_latest` and
  `download_installer` (size and digest mismatch, size-only), `file://`
  opt-in, installer arguments, scope detection against fake registry values,
  and the state machine (once-per-version prompt, `dialog_open`, skip reasons,
  install chaining, hand-off order with injected launch/shutdown closures,
  failure leaves the app running).
- `src/ui/settings.rs` and `src/ui/confirm_dialog.rs` tests cover every phase's
  buttons and ids, toggle persistence, click dispatch and harness layout.
- `pwsh scripts/update-shot.ps1 -Mode dialog|settings|install` runs the real
  binary under a throwaway profile against a `file://` feed advertising
  `v99.0.0` whose asset is a copy of `hostname.exe`: the first two modes
  capture PrintWindow screenshots; `install` asserts the app exits by itself,
  the telemetry chain (`check_completed → download_completed →
  layout_persisted → install_launched → daemon_shutdown ok → exiting`), that
  `workspaces.json` kept its tabs and that the installer landed under the
  profile's `updates` dir. The isolated daemon is confirmed gone by the pipe
  no longer existing.
- `pwsh scripts/update-shot.ps1 -Mode settings -FeedUrl <url>` runs the manual
  check against a real HTTPS feed (the GitHub URL) from a dev build; expect
  `check_completed` with `up_to_date` or `available`. This is what caught the
  `native-tls` feature mistake: `file://` runs never touch the TLS connector.
- `pwsh scripts/update-rehearsal.ps1` exercises the installer half: it derives
  two installers from `packaging/terminal-manager.iss` under a different AppId,
  name and output name (and a harmless `[UninstallRun]` taskkill), installs the
  first into `%LOCALAPPDATA%\tm-rehearsal`, updates it through the app against
  a `file://` feed advertising the second, and asserts the installer log
  (parent wait, both executables free, success, relaunch), the relaunched
  process, `DisplayVersion 99.0.0`, and the telemetry chain from both the old
  and the relaunched app; then it uninstalls and removes every trace. Run it
  after touching `src/updater`, the daemon shutdown or the `.iss` `[Code]`.
- Never run the real installer from a dev tree on a machine with the app
  installed: same AppId, it would replace the user's install. The rehearsal
  script exists so that this is never necessary.

## Boundaries

- Restart-everything: shells and agents die with the daemon. The prompt says so
  and nothing installs without a click.
- No delta updates, no rollback, no code signing check beyond the size and the
  GitHub-published SHA-256 over TLS.
- Release notes open in the browser; nothing is rendered in-app.
- Windows only (registry scope detection, Inno Setup hand-off).

## Open questions

- Whether the daemon can be left running across an update so sessions survive
  (BACKLOG: session-preserving update).
- Whether a copy installed from the non-GPU package should select the
  `non-gpu` asset; the installer does not record the flavour today.

## Decisions log

- Silent installer + relaunch from the installer, not from a helper process:
  Inno already has the process-wait and run-as-original-user primitives, so no
  extra binary is shipped and the app can exit immediately after the launch.
- The daemon is force-stopped by the app before exit rather than by the
  installer, so the shutdown is recorded with the rest of the chain and the
  installer never has to kill anything.
- Startup failures are telemetry-only; a broken network must not toast every
  launch. Manual checks show the error inline in Settings.
- `file://` is gated on `TM_UPDATE_FEED_URL` so e2e runs stay hermetic while the
  production feed cannot be pointed at local files.
- Downloads go to the local (non-roaming) cache dir, not the roaming data dir,
  so a 30 MB installer is never synced by a roaming profile.
