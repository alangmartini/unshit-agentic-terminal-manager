# Desktop Regression Framework

One changelog for all 10 desktop regression worktrees.

## Added

- Added safe self-tests for the desktop regression runner.
- Added a versioned `results.json` shape for desktop test runs.
- Added config profiles and `-ListProfiles`.
- Added preflight checks before real desktop tests run.
- Added owned-process cleanup so normal runs only stop apps they started.
- Added suite metadata validation and tag filtering.
- Added wait/retry helpers so suites do less guessing with sleeps.
- Added authoring helpers for app sessions, before/after screenshots, geometry
  checks, and visual stripe checks.
- Added failure diagnostics: manifest, final screenshot, foreground window,
  active app session, and run log links.

## Changed

- Updated the existing desktop suites to use the shared helpers.
- Updated desktop regression wrappers to pass through the new runner options.

## Notes

- Full desktop suites are still manual. They launch the app, move the mouse,
  send global keys, and capture the screen.
