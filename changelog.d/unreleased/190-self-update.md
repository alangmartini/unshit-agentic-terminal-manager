### Added

- **Self-update.** The app checks GitHub Releases shortly after startup and,
  once per new version, offers to install it; **Settings › Updates** shows the
  running version with *check for updates*, *install and restart* and a switch
  for the startup check. Installing downloads the release installer, verifies
  its size and SHA-256 digest, saves the workspace layout, stops the session
  daemon and hands off to the installer, which waits for the app to exit,
  installs silently and relaunches it. Workspaces and tabs come back with
  fresh shells; the prompt says so and nothing installs without a click.
  Copies not set up by the installer (source builds) get *open release page*
  instead. Dev and test profiles never poll GitHub unless
  `TM_UPDATE_FEED_URL` points them at a feed; `TM_UPDATE_STARTUP_DELAY_MS`
  and `TM_UPDATE_INSTALL_SCOPE` cover the other knobs. Every step is
  recorded in the profile's `update-events.jsonl`. The installer side of the
  hand-off ships with this release and is first used when the *next* release
  is installed through the app.
