# Desktop Interaction Regression

Desktop Interaction Regression (DIR) suites exercise behavior that depends on a real Windows desktop: native window bounds, compositor snap, focus, synthesized input, screenshots, DPI, and pixel-level visual checks. They are not unit tests and not browser e2e tests.

## Commands

Run every suite sequentially:

```powershell
powershell.exe -ExecutionPolicy Bypass -File scripts\desktop-regression\run.ps1
```

Run every suite against an already-built binary:

```powershell
powershell.exe -ExecutionPolicy Bypass -File scripts\desktop-regression\run.ps1 -SkipBuild
```

Run one suite:

```powershell
powershell.exe -ExecutionPolicy Bypass -File scripts\desktop-regression\run.ps1 -Suite post-resize-glitches
```

List suites and what they cover:

```powershell
powershell.exe -ExecutionPolicy Bypass -File scripts\desktop-regression\run.ps1 -List
```

The old resize command is kept as a wrapper:

```powershell
powershell.exe -ExecutionPolicy Bypass -File scripts\window-resize-automation.ps1 -OnlySnapTest
```

## Suites

- `post-resize-glitches`: covers Aero snap growth, bottom statusbar reflow, and stale terminal rows floating in the enlarged viewport.
- `edge-resize-stability`: covers frameless left-edge drag behavior and right-edge stability.

## Adding A Suite

1. Copy `templates/suite.ps1` to `suites/<suite-name>.ps1`.
2. Set `-Name`, `-Title`, `-Covers`, and `-Tags`.
3. Put the scenario in the registered `-ScriptBlock`.
4. Use helpers from `lib/DesktopRegression.ps1` for app launch, window focus, resize/snap input, screenshots, pixel sampling, and assertions.
5. Run it directly:

```powershell
powershell.exe -ExecutionPolicy Bypass -File scripts\desktop-regression\run.ps1 -Suite <suite-name>
```

Suites should be named after the behavior they protect, not after the bug number that motivated them.
