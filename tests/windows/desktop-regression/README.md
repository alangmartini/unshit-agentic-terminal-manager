# Desktop Interaction Regression Framework

Desktop Interaction Regression (DIR) suites exercise behavior that depends on a
real Windows desktop: native window bounds, compositor snap, focus, synthesized
input, screenshots, DPI, and pixel-level visual checks. They spawn
`terminal-manager.exe` and drive it like a user would.

These are manual desktop tests. They are not unit tests, browser e2e tests, or
CI jobs.

For a detailed execution and debugging runbook, see
`docs/desktop-regression-debugging.md`.

## Commands

The canonical runner is the Rust `xtask` command. List suites and coverage:

```powershell
cargo xtask desktop-regression --list
```

Run every suite sequentially:

```powershell
cargo xtask desktop-regression
```

Run one suite:

```powershell
cargo xtask desktop-regression --suite post-resize-glitches
```

Run one suite against an already-built binary:

```powershell
cargo xtask desktop-regression --suite edge-resize-stability --skip-build --exe-path target\debug\terminal-manager.exe
```

Choose observability level:

```powershell
cargo xtask desktop-regression --suite post-resize-glitches --observe off
cargo xtask desktop-regression --suite post-resize-glitches --observe basic
cargo xtask desktop-regression --suite post-resize-glitches --observe full
```

`--observe off` keeps the run black-box and does not enable app diagnostics.
`--observe basic` enables diagnostics for handshake, logs, events, and failure
evidence. `--observe full` also records step snapshots, invariants, and
cross-layer assertions where the suite supports them.

The current app diagnostic stream advertises the event families it actually
emits: `test_step`, `invariant`, and `log`. Window, layout, render, terminal,
PTY, and input events should be wired before those families are advertised.

Artifacts are written under `artifacts/windows/desktop-regression/<run_id>/`
by default. Use `--artifact-root <dir>` or `--artifacts-root <dir>` to choose a
different Rust artifact root.

Record and replay an edge-resize interaction trace:

```powershell
cargo xtask desktop-regression --suite edge-resize-stability --record
cargo xtask desktop-regression --suite edge-resize-stability --replay artifacts\windows\desktop-regression\<run_id>\runner.actions.jsonl
```

Replay is currently logical replay for `edge-resize-stability`: runner input
actions are re-applied to a fresh app process, and recorded `resize_window`
actions are checked as assertions.

## PowerShell Compatibility

The historical PowerShell runner is now a compatibility wrapper around
`cargo xtask desktop-regression`. It remains useful for old commands:

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

The wrapper maps repeated `-Suite`, `-SkipBuild`, `-ExePath`, `-Observe`,
`-Interactive`, `-KeepOpenOnFailure`, `-Record`, and explicit `-ArtifactsDir`
to Rust arguments. Legacy `-ArtifactsDir <dir>` keeps its old meaning and maps
to Rust artifact root `<dir>\windows\desktop-regression`.

PowerShell-only tuning flags such as `-Tolerance`, `-DragDelta`, and `-Snap*`
are accepted for script compatibility but ignored with a warning because the
migrated Rust suites own their thresholds and fixtures. No legacy-only
PowerShell suites remain; both current suite ids run through Rust.

Historical script paths still work as compatibility wrappers:

```powershell
powershell.exe -ExecutionPolicy Bypass -File scripts\desktop-regression\run.ps1 -List
powershell.exe -ExecutionPolicy Bypass -File scripts\window-resize-automation.ps1 -OnlySnapTest
```

## Framework Layout

- `run.ps1`: PowerShell compatibility wrapper that forwards to the Rust runner.
- `xtask/src/desktop_regression/`: canonical suite registry, CLI parsing,
  build/launch orchestration, observability, artifacts, and result collection.
- `lib/DesktopRegression.ps1`: Win32 driver, process/window lifecycle,
  screenshot capture, pixel sampling, and assertion helpers kept for historical
  reference and compatibility.
- `suites/*.ps1`: legacy registered desktop scenarios retained while old notes
  still point at this directory.
- `templates/suite.ps1`: starting point for a new suite.
- `SPEC.md`: framework contract and boundaries.

## Current Suites

- `post-resize-glitches`: covers Aero snap growth, bottom statusbar reflow, and
  stale terminal rows floating in the enlarged viewport.
- `edge-resize-stability`: covers frameless left-edge drag behavior and
  right-edge stability.

## Adding A Suite

1. Add a Rust suite module under `xtask/src/desktop_regression/suites/`.
2. Register its metadata in `xtask/src/desktop_regression/registry.rs`.
3. Use the Rust desktop-regression helpers for app launch, window focus,
   resize/snap input, screenshots, diagnostics, pixel sampling, and assertions.
4. Add or update focused `xtask` tests for registry, options, result, or helper
   behavior changed by the suite.
5. Run it directly:

```powershell
cargo xtask desktop-regression --suite <suite-name>
```

Suites should be named after the behavior they protect, not after the bug
number that motivated them.
