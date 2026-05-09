# Desktop Interaction Regression Framework

Desktop Interaction Regression (DIR) suites exercise behavior that depends on a
real Windows desktop: native window bounds, compositor snap, focus, synthesized
input, screenshots, DPI, and pixel-level visual checks. They spawn
`terminal-manager.exe` and drive it like a user would.

These are manual desktop tests. They are not unit tests, browser e2e tests, or
CI jobs.

## Commands

List suites and coverage:

```powershell
powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -List
```

Run every suite sequentially:

```powershell
powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1
```

Run every suite against an already-built binary:

```powershell
powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -SkipBuild
```

Run one suite:

```powershell
powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Suite post-resize-glitches
```

Historical script paths still work as compatibility wrappers:

```powershell
powershell.exe -ExecutionPolicy Bypass -File scripts\desktop-regression\run.ps1 -List
powershell.exe -ExecutionPolicy Bypass -File scripts\window-resize-automation.ps1 -OnlySnapTest
```

## Framework Layout

- `run.ps1`: suite discovery, build orchestration, execution, and result
  collection.
- `lib/DesktopRegression.ps1`: Win32 driver, process/window lifecycle,
  screenshot capture, pixel sampling, and assertion helpers.
- `suites/*.ps1`: registered desktop scenarios.
- `templates/suite.ps1`: starting point for a new suite.
- `SPEC.md`: framework contract and boundaries.

## Current Suites

- `post-resize-glitches`: covers Aero snap growth, bottom statusbar reflow, and
  stale terminal rows floating in the enlarged viewport.
- `edge-resize-stability`: covers frameless left-edge drag behavior and
  right-edge stability.

## Adding A Suite

1. Copy `templates/suite.ps1` to `suites/<suite-name>.ps1`.
2. Set `-Name`, `-Title`, `-Covers`, and `-Tags`.
3. Put the scenario in the registered `-ScriptBlock`.
4. Use helpers from `lib/DesktopRegression.ps1` for app launch, window focus,
   resize/snap input, screenshots, pixel sampling, and assertions.
5. Run it directly:

```powershell
powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Suite <suite-name>
```

Suites should be named after the behavior they protect, not after the bug
number that motivated them.
