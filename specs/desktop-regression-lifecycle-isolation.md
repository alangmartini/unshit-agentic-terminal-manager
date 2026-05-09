# Spec: Desktop Regression Lifecycle Isolation

## Objective

Strengthen lifecycle isolation for `tests/windows/desktop-regression` so a
desktop regression run owns and cleans up only the `terminal-manager.exe`
processes it starts by default.

The current helper path launches test sessions through
`Start-DesktopRegressionApp`, which calls `Stop-DesktopRegressionStaleApps`
before every app launch. That stale cleanup uses the broad process name
`terminal-manager`, so an unrelated user-owned development instance can be
terminated before a manual desktop test starts. The new default must replace
that broad cleanup with per-run process ownership, PID tracking, safer cleanup,
and opt-in stale process handling with clear warnings for destructive cleanup.

Success means engineers can run the manual desktop suites while another
`terminal-manager.exe` process is open, and the framework only terminates
processes it started for the current run unless the operator explicitly opts
into stale cleanup.

## Tech Stack

- PowerShell 5.1 runner and helper modules.
- Windows desktop APIs loaded by `Add-Type` in
  `tests/windows/desktop-regression/lib/DesktopRegression.ps1`.
- `terminal-manager.exe` launched from `target/debug/terminal-manager.exe` by
  default after `cargo build`.
- JSON artifacts written with `ConvertTo-Json` under
  `artifacts/windows/desktop-regression/<run_id>/`.
- Existing manual desktop suite registration through
  `Register-DesktopRegressionSuite`.

## Commands

- List desktop regression suites without launching the app:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -List`
- Run all suites with the default owned-process lifecycle:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1`
- Run one suite with the default owned-process lifecycle:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Suite post-resize-glitches`
- Run against an existing binary without rebuilding:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -SkipBuild`
- Proposed explicit stale-process cleanup before the run:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -CleanStaleApps`
- Proposed stale-process report without termination:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -ReportStaleApps`
- Proposed destructive cleanup confirmation bypass for automation or deliberate
  local use:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -CleanStaleApps -Force`

## Project Structure

- `tests/windows/desktop-regression/run.ps1`: command-line parameters, suite
  discovery, build orchestration, context creation, result writing, and
  run-level final cleanup.
- `tests/windows/desktop-regression/lib/DesktopRegression.ps1`: shared Win32
  helpers, process launch, PID ownership tracking, process cleanup, stale
  process reporting, and assertions.
- `tests/windows/desktop-regression/SPEC.md`: framework contract; update it to
  state that broad cleanup is no longer the default lifecycle behavior.
- `tests/windows/desktop-regression/README.md`: operator-facing commands and
  warnings for manual desktop runs.
- `tests/windows/desktop-regression/suites/*.ps1`: suites continue to use
  `Start-DesktopRegressionApp` and `Stop-DesktopRegressionApp`; they should not
  implement ad hoc process cleanup.
- `artifacts/windows/desktop-regression/<run_id>/owned-processes.json`:
  proposed per-run ownership ledger containing launched PIDs and cleanup status.

## Code Style

Use explicit ownership checks and idempotent cleanup helpers. Avoid name-based
process termination in the default path. Keep PowerShell strict-mode compatible
and preserve the existing verb-prefixed helper naming style.

```powershell
function Register-DesktopRegressionOwnedProcess {
    param(
        [Parameter(Mandatory = $true)]$Context,
        [Parameter(Mandatory = $true)][System.Diagnostics.Process]$Process
    )

    if (-not $Context.OwnedProcessIds.Contains($Process.Id)) {
        [void]$Context.OwnedProcessIds.Add($Process.Id)
    }
}

function Stop-DesktopRegressionOwnedProcess {
    param(
        [Parameter(Mandatory = $true)]$Context,
        [Parameter(Mandatory = $true)][int]$ProcessId
    )

    if (-not $Context.OwnedProcessIds.Contains($ProcessId)) {
        Write-Warning "Refusing to stop unowned terminal-manager process pid=$ProcessId"
        return
    }

    $proc = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
    if ($proc -and -not $proc.HasExited) {
        Stop-Process -Id $ProcessId -ErrorAction SilentlyContinue
    }
}
```

Conventions:

- Store owned PIDs on the context returned by `New-DesktopRegressionContext`.
- Register a PID immediately after `Start-Process -PassThru` succeeds.
- Use `Write-Warning` for any cleanup mode that can affect processes outside
  the current run.
- Keep destructive cleanup behind explicit runner parameters, not suite code.
- Prefer helper functions in `lib/DesktopRegression.ps1` over repeated cleanup
  logic in `run.ps1` or individual suites.

## Testing Strategy

- Unit-style PowerShell checks for helper behavior should cover PID
  registration, refusal to stop unowned PIDs, cleanup idempotency, and ledger
  serialization without driving the desktop.
- `-List` must remain non-invasive and must not call stale cleanup, launch
  `terminal-manager.exe`, or write an ownership ledger.
- Manual smoke test with a separate `terminal-manager.exe` already running:
  run one desktop suite and verify the unrelated process remains alive.
- Manual failure-path test: force a suite failure after launch and verify the
  run-level cleanup still stops owned PIDs and records cleanup status.
- Stale-process option test: run `-ReportStaleApps` and confirm it prints
  candidates without termination; run `-CleanStaleApps` only after the command
  prints an explicit warning or requires `-Force`.
- Full suite execution remains manual because the framework controls the real
  desktop, sends global input such as `Win+Left`, moves the mouse, and captures
  the screen.

## Boundaries

- Always: default to current-run PID ownership; clean up owned processes in
  suite `finally` blocks and in a run-level `finally`; write enough artifact
  state to diagnose leaked owned PIDs.
- Always: make destructive stale cleanup visibly opt-in with warnings that it
  can terminate unrelated local development processes.
- Always: keep `tests/windows/desktop-regression` as the canonical framework
  path and keep compatibility wrappers pointing to it if they already exist.
- Ask first: changing public runner parameter names after they are documented;
  adding dependencies beyond built-in PowerShell/.NET APIs; adding these manual
  suites to CI/CD.
- Ask first: changing suite behavior outside lifecycle cleanup, including input
  timing, screenshot assertions, or pixel thresholds.
- Never: stop processes by broad name in the default run path; terminate an
  unowned PID without explicit stale-cleanup controls; edit unrelated specs or
  other workers' files.

## Plan

1. Add lifecycle options to `run.ps1`.
   - Introduce explicit controls such as `-ReportStaleApps`,
     `-CleanStaleApps`, and `-Force`.
   - Keep the no-flag default non-destructive.
   - Print warnings before any stale-process termination.

2. Extend `New-DesktopRegressionContext` with ownership state.
   - Add an owned PID collection and an ownership ledger path under the current
     run artifact directory.
   - Ensure the context still exposes the existing fields used by suites.

3. Replace broad default cleanup in `Start-DesktopRegressionApp`.
   - Remove the unconditional `Stop-DesktopRegressionStaleApps` call from the
     launch path.
   - Register the launched process as owned immediately after `Start-Process`
     succeeds.
   - Return the same session shape, with process and window handle fields, so
     suites remain compatible.

4. Make cleanup layered and idempotent.
   - Keep `Stop-DesktopRegressionApp` safe for per-session `finally` blocks.
   - Add run-level cleanup in `run.ps1` to catch owned processes that survive a
     suite failure or missing suite cleanup.
   - Update the ownership ledger with launched, stopped, missing, and failed
     cleanup states.

5. Implement opt-in stale handling.
   - Preserve name-based discovery only for report/explicit cleanup modes.
   - Exclude owned current-run PIDs from stale candidates.
   - Require either interactive confirmation or `-Force` before termination
     when `-CleanStaleApps` is used.

6. Update framework documentation.
   - Document the new default lifecycle behavior in `SPEC.md`.
   - Add README commands and warnings for reporting or cleaning stale processes.
   - Keep existing documented run commands valid.

## Tasks

- [ ] Task: Add explicit lifecycle parameters to the runner.
  - Acceptance: `run.ps1` accepts report, clean, and force controls while
    existing commands continue to parse.
  - Verify:
    `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -List`
  - Files: `tests/windows/desktop-regression/run.ps1`

- [ ] Task: Add owned-process tracking to the desktop regression context.
  - Acceptance: every run context has an owned PID collection and an
    `owned-processes.json` path inside the run artifact directory.
  - Verify: run one suite with `-SkipBuild` and confirm the ledger is created
    under `artifacts/windows/desktop-regression/<run_id>/`.
  - Files: `tests/windows/desktop-regression/lib/DesktopRegression.ps1`

- [ ] Task: Remove broad stale cleanup from the default launch path.
  - Acceptance: `Start-DesktopRegressionApp` no longer calls
    `Stop-DesktopRegressionStaleApps` before launching and registers only the
    newly started PID as owned.
  - Verify: start an unrelated `terminal-manager.exe`, run one suite, and
    confirm the unrelated PID remains alive.
  - Files: `tests/windows/desktop-regression/lib/DesktopRegression.ps1`

- [ ] Task: Add safe owned-process cleanup helpers.
  - Acceptance: cleanup stops owned live PIDs, ignores already exited PIDs, and
    warns/refuses when asked to stop an unowned PID through the owned cleanup
    path.
  - Verify: run helper-level checks or a manual failure-path suite and inspect
    warnings plus `owned-processes.json`.
  - Files: `tests/windows/desktop-regression/lib/DesktopRegression.ps1`

- [ ] Task: Add run-level final cleanup.
  - Acceptance: even if a suite throws before its own cleanup completes, the
    runner attempts to stop all owned current-run processes before writing final
    pass/fail output.
  - Verify: force a controlled suite failure after app launch and confirm no
    owned `terminal-manager.exe` PID is left running.
  - Files: `tests/windows/desktop-regression/run.ps1`,
    `tests/windows/desktop-regression/lib/DesktopRegression.ps1`

- [ ] Task: Implement stale-process report and opt-in cleanup.
  - Acceptance: report mode lists stale candidates without terminating them;
    cleanup mode prints a destructive-action warning and requires confirmation
    or `-Force`.
  - Verify:
    `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -ReportStaleApps -List`
    does not stop any process; a deliberate `-CleanStaleApps -Force` run stops
    only listed stale candidates.
  - Files: `tests/windows/desktop-regression/run.ps1`,
    `tests/windows/desktop-regression/lib/DesktopRegression.ps1`

- [ ] Task: Update desktop regression docs.
  - Acceptance: `SPEC.md` and `README.md` explain owned-process defaults,
    stale-process controls, and warnings for destructive cleanup.
  - Verify: documentation includes the existing basic commands plus the new
    report/cleanup commands.
  - Files: `tests/windows/desktop-regression/SPEC.md`,
    `tests/windows/desktop-regression/README.md`

## Success Criteria

- Running the desktop regression framework without stale-cleanup flags never
  stops `terminal-manager.exe` processes that were not launched by the current
  run.
- Every launched test app PID is tracked in the run context and reflected in
  `owned-processes.json`.
- Suite-level cleanup and run-level cleanup are both idempotent and safe to call
  after partial failures.
- `-List` remains non-invasive.
- Operators have a report-only stale process mode and an explicit destructive
  stale cleanup mode with clear warnings and force/confirmation controls.
- Existing suite authoring guidance remains valid: suites launch through
  `Start-DesktopRegressionApp` and clean up with `Stop-DesktopRegressionApp` in
  `finally`.

## Open Questions

- Should `-CleanStaleApps` require interactive confirmation by default, or is a
  warning plus `-Force` requirement enough for this manual test framework?
- Should stale detection be limited to executable path equality with
  `$Context.ExePath`, or should it include any process named
  `terminal-manager`?
- Should the ownership ledger be updated incrementally during the run, or only
  written once during final cleanup?
- Should leaked owned PIDs after failed cleanup make the overall run fail even
  if all suites passed?
