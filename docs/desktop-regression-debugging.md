# Desktop Regression Debugging Runbook

This runbook is for future sessions investigating the Windows desktop
regression harness. It covers execution, artifact inspection, diagnostic
snapshots, app inner state, replay, and failure triage.

## What This Harness Is

The canonical runner is:

```powershell
cargo xtask desktop-regression
```

It launches `terminal-manager.exe`, drives a real Windows desktop with Win32
window operations, global input, screenshots, and optional app diagnostics. The
current Rust-backed suites are:

- `edge-resize-stability`
- `post-resize-glitches`

These are manual headed tests. They move windows, move the mouse, send global
input, and capture the screen. Do not treat them as CI-safe unless that is
explicitly requested later.

## Cargo Setup On This Machine

The normal `cargo` shim may fail if `rustup.exe` is missing. Use the stable
toolchain binary directly from the repo root:

```powershell
cd C:\Users\Alan Beelink\dev\unshit-test-framework

$cargo = Join-Path $env:USERPROFILE '.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin\cargo.exe'
$toolchainBin = Split-Path -Parent $cargo
$env:RUSTC = Join-Path $toolchainBin 'rustc.exe'
$env:RUSTDOC = Join-Path $toolchainBin 'rustdoc.exe'
$env:PATH = "$toolchainBin;$env:PATH"
```

Then use `& $cargo ...` in place of `cargo ...`.

## Non-Invasive Validation

Run these before touching the real desktop:

```powershell
& $cargo fmt --check
& $cargo test -p terminal-manager-diagnostics
& $cargo test -p xtask desktop_regression
& $cargo test --bin terminal-manager diagnostics
& $cargo build --bin terminal-manager
& $cargo xtask desktop-regression --list
```

Compatibility wrapper smoke checks:

```powershell
powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -List
powershell.exe -ExecutionPolicy Bypass -File scripts\desktop-regression\run.ps1 -List
```

## Headed Execution

Run one suite against an already-built binary:

```powershell
& $cargo xtask desktop-regression --suite edge-resize-stability --observe off --skip-build --exe-path target\debug\terminal-manager.exe
```

Run with full diagnostics:

```powershell
& $cargo xtask desktop-regression --suite edge-resize-stability --observe full --skip-build --exe-path target\debug\terminal-manager.exe
```

Run all registered suites:

```powershell
& $cargo xtask desktop-regression
```

Use `--skip-build --exe-path target\debug\terminal-manager.exe` when the binary
is already built and you want faster iteration.

## Observe Modes

- `--observe off`: black-box desktop run. No app diagnostic endpoint.
- `--observe basic`: diagnostic handshake, final snapshot, event/log drain, and
  failure evidence, plus suite step snapshots such as `pre-snap` / `post-snap`
  so cross-layer assertions can run where suites use them.
- `--observe full`: basic mode plus deterministic-mode preparation, step
  markers, invariant evaluation, and any extra full-only checks.

The app diagnostic endpoint is gated by environment variables set by the
runner:

- `TM_DIAGNOSTICS_ENABLE=1`
- `TM_DIAGNOSTICS_PIPE_NAME=<per-run named pipe>`
- `TM_DIAGNOSTICS_TOKEN=<per-run token>`

The app currently advertises and emits only these diagnostic event families:

- `test_step`
- `invariant`
- `log`

Do not assume `window`, `layout`, `render`, `terminal`, `pty`, or `input` event
streams exist yet. Those areas are visible in snapshots, but their ordered event
producers still need to be wired.

## Artifacts

Each run writes:

```text
artifacts/windows/desktop-regression/<run_id>/
```

Run ids include UTC timestamp, milliseconds, and process id for collision
resistance.

Common files:

- `results.json`: primary machine-readable result summary.
- `environment.json`: source and binary metadata, including binary hash.
- `runner.events.jsonl`: runner lifecycle and artifact-write events.
- `<suite>-app.stdout.log`: app stdout for that suite process.
- `<suite>-app.stderr.log`: app stderr for that suite process.
- `<suite>-*.png`: screenshots captured by the suite or failure bundle.
- `<suite>-failure-manifest.json`: failure classification and linked evidence,
  present on failed suites.

When `--record` is used:

- `runner.actions.jsonl`: replayable runner actions with timestamps, targets,
  step ids, and coordinates.

When diagnostics are enabled:

- `<suite>-diagnostics-hello.json`: protocol, app identity, capabilities, pid.
- `<suite>-diagnostics-final-snapshot.json`: final app state snapshot.
- `<suite>-diagnostics-failure-snapshot.json`: failure snapshot when applicable.
- `<suite>-diagnostics-invariants.json`: invariant results for `--observe full`.
- `<suite>-diagnostics-summary.json`: flushed/drained event counts.
- `<suite>-app.events.jsonl`: drained diagnostic events.
- `<suite>-app.logs.jsonl`: drained diagnostic events with `family=log`.
- `<suite>-*-snapshot.json`: step snapshots where the suite captures them.

## Inspect Results Quickly

Set a run directory:

```powershell
$run = 'artifacts\windows\desktop-regression\<run_id>'
$result = Get-Content "$run\results.json" | ConvertFrom-Json
```

Run status and selected suites:

```powershell
$result.run
$result.summary
$result.suites | Select-Object id,status
```

Failure details:

```powershell
$result.suites |
  Where-Object status -eq 'failed' |
  Select-Object id,@{n='kind';e={$_.failure.kind}},@{n='message';e={$_.failure.message}},@{n='signal';e={$_.failure.first_bad_signal}}
```

Artifacts linked to one suite:

```powershell
($result.suites | Where-Object id -eq 'edge-resize-stability').artifacts
```

Recorded or replayed actions:

```powershell
($result.suites | Where-Object id -eq 'edge-resize-stability').actions |
  Select-Object seq,step_id,target,kind
```

## Inspect App Inner State

Use `--observe basic` or `--observe full` when you need app inner state. The
important snapshot file is usually:

```text
<suite>-diagnostics-final-snapshot.json
```

On failures, prefer:

```text
<suite>-diagnostics-failure-snapshot.json
```

Load a snapshot:

```powershell
$snapshot = Get-Content "$run\edge-resize-stability-diagnostics-final-snapshot.json" | ConvertFrom-Json
```

Important snapshot areas:

- `$snapshot.app`: app pid, build identity, diagnostic endpoint.
- `$snapshot.window`: reported window bounds, focus, scale factor, resize
  generation when available.
- `$snapshot.layout`: layout nodes, bounds, visibility, z-order, dirty state.
- `$snapshot.terminal`: grid rows/cols, visible rows, cursor, scrollback
  length, active session id, and the optional `buffer_window` only when a
  snapshot request explicitly opts in to terminal contents.
- `$snapshot.renderer`: surface size, frame counter, last-present time, dirty
  cell regions, cached layers, and render error fields. Glyph atlas page counts
  are not wired yet because the current app state does not expose them.
- `$snapshot.pty`: pty sessions, pending writes, and content-free recent pty
  liveness events.
- `$snapshot.input`: focused element, pointer/drag state, modifiers.
- `$snapshot.config`: config values that affect rendering and layout.
- `$snapshot.recent_errors`: app-side errors captured in diagnostic state.

Useful checks:

```powershell
$snapshot.app
$snapshot.window
$snapshot.terminal.grid
$snapshot.renderer.surface_size
$snapshot.layout.nodes | Select-Object id,label,bounds,visible,z_order,dirty
$snapshot.pty.sessions | Select-Object id,name,process_id,status,reconnecting
$snapshot.recent_errors
```

Invariant results:

```powershell
$invariants = Get-Content "$run\edge-resize-stability-diagnostics-invariants.json" | ConvertFrom-Json
$invariants | Select-Object id,outcome,message
$invariants | Where-Object outcome -eq 'failed'
```

Diagnostic events and logs:

```powershell
Get-Content "$run\edge-resize-stability-app.events.jsonl" | Select-Object -First 20
Get-Content "$run\edge-resize-stability-app.logs.jsonl" | Select-Object -First 20
```

The event files are JSONL. Convert a small sample when needed:

```powershell
Get-Content "$run\edge-resize-stability-app.events.jsonl" |
  Select-Object -First 20 |
  ForEach-Object { $_ | ConvertFrom-Json } |
  Select-Object seq,test_step_id,payload
```

`DrainEvents` consumes the app event queue. If an interactive session drains
events before finalization, later event artifacts may contain only events
recorded after that drain.

## Interactive Failure Debugging

Use interactive mode to keep the app alive after a failure:

```powershell
& $cargo xtask desktop-regression --suite edge-resize-stability --observe full --interactive --keep-open-on-failure --skip-build --exe-path target\debug\terminal-manager.exe
```

Prompt commands:

- `snapshot`: write an interactive diagnostic snapshot.
- `events` or `tail`: drain current diagnostic events to JSONL.
- `screenshot`: capture a screen screenshot.
- `note <text>`: add a human debugging note to the run artifacts.
- `rerun`: currently records that rerun is unsupported in interactive v1.
- `continue`: continue suite cleanup and proceed.
- `abort`: abort the remaining run after cleanup.
- `close`: explicitly close the app, then finish.

To force the interactive workflow without waiting for a real regression:

```powershell
$env:TM_DESKTOP_REGRESSION_FORCE_FAILURE = 'edge-resize-stability'
& $cargo xtask desktop-regression --suite edge-resize-stability --observe full --interactive --keep-open-on-failure --skip-build --exe-path target\debug\terminal-manager.exe
Remove-Item Env:\TM_DESKTOP_REGRESSION_FORCE_FAILURE
```

## Record And Replay

Record an action trace:

```powershell
& $cargo xtask desktop-regression --suite edge-resize-stability --observe off --record --skip-build --exe-path target\debug\terminal-manager.exe
```

Replay it:

```powershell
& $cargo xtask desktop-regression --suite edge-resize-stability --observe off --replay artifacts\windows\desktop-regression\<run_id>\runner.actions.jsonl --skip-build --exe-path target\debug\terminal-manager.exe
```

Current replay behavior:

- Replay is logical, not exact timed replay.
- Supported suite: `edge-resize-stability`.
- Input actions such as move, click, wait, drag, screenshot, and simple
  send-keys actions are consumed by the replay runner.
- Recorded `resize_window` actions are assertions against the fresh app process.
- Unsupported suites are rejected before a misleading replay result is written.

Inspect a trace:

```powershell
Get-Content "$run\runner.actions.jsonl" |
  ForEach-Object { $_ | ConvertFrom-Json } |
  Select-Object seq,suite_id,step_id,target,kind
```

## Failure Triage Checklist

Start with `results.json`:

1. Check `run.status`, `summary`, and each suite status.
2. For failed suites, read `failure.kind`, `failure.message`, and
   `failure.first_bad_signal`.
3. Open `<suite>-failure-manifest.json` and follow the linked artifacts.
4. Compare screenshots around the failing step.
5. Check app stderr/stdout logs.
6. If diagnostics were enabled, inspect hello, snapshots, invariants, events,
   and diagnostics summary.
7. If the issue is timing-sensitive, rerun with `--record` and replay the trace.
8. If the issue needs live inspection, rerun with `--observe full --interactive
   --keep-open-on-failure`.

Failure classifications are intended to separate setup, protocol, assertion,
visual regression, cross-layer invariant, timeout, app crash, artifact, and
unknown failures. A setup failure usually means the harness or environment
failed. Assertion, visual regression, and cross-layer invariant failures usually
mean the app behavior or reported state disagreed with the suite contract.

## Code Map

Runner:

- `xtask/src/desktop_regression/options.rs`: CLI options.
- `xtask/src/desktop_regression/registry.rs`: suite metadata and selection.
- `xtask/src/desktop_regression/runner.rs`: top-level orchestration.
- `xtask/src/desktop_regression/suites/`: suite implementations.
- `xtask/src/desktop_regression/win32.rs`: headed desktop operations.
- `xtask/src/desktop_regression/diagnostics.rs`: harness diagnostic client.
- `xtask/src/desktop_regression/replay.rs`: action recording and trace
  validation.
- `xtask/src/desktop_regression/results.rs`: `results.json` assembly.
- `xtask/src/desktop_regression/failure.rs`: failure bundles and manifests.
- `xtask/src/desktop_regression/interactive.rs`: interactive failure prompt.

App diagnostics:

- `src/diagnostics/config.rs`: environment gates.
- `src/diagnostics/server.rs`: command handling and capability reporting.
- `src/diagnostics/transport.rs`: Windows named pipe transport.
- `src/diagnostics/snapshot.rs`: app state snapshot and invariants.
- `src/diagnostics/events.rs`: in-memory diagnostic event store.
- `crates/terminal-manager-diagnostics/src/lib.rs`: shared protocol and result
  schemas.

Compatibility entry points:

- `tests/windows/desktop-regression/run.ps1`
- `scripts/desktop-regression/run.ps1`
- `scripts/window-resize-automation.ps1`

## Known Limits

- The desktop suites are Windows-only and headed.
- Full event instrumentation is not complete. Snapshot producer fields are wired
  for active terminal cursor/scrollback/session, PTY mappings/recent liveness,
  and renderer frame presence, but only `test_step`, `invariant`, and `log`
  diagnostic events are currently emitted.
- Replay is logical and currently implemented for `edge-resize-stability`.
- Interactive `rerun` is a placeholder in v1.
- The diagnostic endpoint is intentionally unavailable unless the runner sets
  the explicit environment gates and per-run token.

## When Adding New Suites

1. Add a Rust module under `xtask/src/desktop_regression/suites/`.
2. Register metadata in `xtask/src/desktop_regression/registry.rs`.
3. Use shared helpers for launch, Win32 operations, screenshots, diagnostics,
   failure artifacts, and assertions.
4. Give assertions clear `first_bad_signal` values.
5. Add focused unit tests for changed registry, option, replay, result, or helper
   behavior.
6. Update `tests/windows/desktop-regression/README.md` and this runbook when
   execution or artifacts change.
