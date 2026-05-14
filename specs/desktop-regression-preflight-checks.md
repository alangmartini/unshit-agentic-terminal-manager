# Spec: Desktop Regression Preflight Checks

## Objective

Add a serious framework-grade preflight mode for `tests/windows/desktop-regression`
so engineers and agents can verify that a Windows desktop machine is suitable
before a suite takes over the mouse, keyboard, compositor, and screen capture
state.

The primary user is anyone running the manual Desktop Interaction Regression
framework described in `tests/windows/desktop-regression/SPEC.md` and
`tests/windows/desktop-regression/README.md`. The mode should make environment
problems explicit before running suites, especially problems that currently fail
late or non-obviously inside `run.ps1` or `lib/DesktopRegression.ps1`.

The proposed CLI surface is `-Check` on
`tests/windows/desktop-regression/run.ps1`. In check mode, the runner performs
all validations, prints a structured readiness report, writes an optional
artifact summary, and exits without launching suites unless the caller also
requests normal execution after a successful check.

The check must cover:

- Windows version and desktop capability.
- PowerShell version and edition compatibility.
- Screen bounds, DPI awareness, and display scaling state.
- Shell availability for the configured `-SnapShell`.
- Binary path resolution, build intent, and file accessibility.
- Permissions and interactive desktop access for Win32 input, focus, and
  screenshots.
- Stale `terminal-manager` process detection and safe handling before suite
  execution.

## Tech Stack

- Windows PowerShell 5.1+, matching the current `#Requires -Version 5.1`
  headers in `run.ps1` and `lib/DesktopRegression.ps1`.
- PowerShell Core 7.x may be detected and reported, but the canonical command
  remains `powershell.exe` until compatibility is deliberately expanded.
- Win32 interop through `Add-Type` in `Initialize-DesktopRegressionWin32`.
- .NET `System.Drawing` and `System.Windows.Forms` assemblies for screenshots
  and `SendKeys`.
- Existing Rust build output at `target\debug\terminal-manager.exe` unless
  `-ExePath` overrides it.
- Existing desktop regression artifact layout under
  `artifacts/windows/desktop-regression/<run_id>/`.

## Commands

- List suites:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -List`
- Run preflight only:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Check`
- Run preflight against an existing binary:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Check -SkipBuild -ExePath target\debug\terminal-manager.exe`
- Run preflight with a specific shell profile:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Check -SnapShell bash`
- Run all suites after normal build and validation:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1`
- Run one suite after normal build and validation:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Suite post-resize-glitches`

`-List` stays non-invasive and should not require the full preflight. `-Check`
must not move the mouse, send keys, snap windows, or run suite scriptblocks.

## Project Structure

- `tests/windows/desktop-regression/run.ps1`: add the `-Check` switch, call
  preflight after suite registration and path normalization, and stop before
  suite execution when check-only mode is requested.
- `tests/windows/desktop-regression/lib/DesktopRegression.ps1`: add shared
  preflight helpers for OS, PowerShell, screen, shell, binary, permissions,
  interactive desktop, and stale process checks.
- `tests/windows/desktop-regression/SPEC.md`: update the framework contract to
  document that preflight is the supported readiness gate for manual desktop
  suites.
- `tests/windows/desktop-regression/README.md`: document check commands,
  report semantics, and how to resolve common failures.
- `artifacts/windows/desktop-regression/<run_id>/preflight.json`: optional
  machine-readable report when an artifact directory is created.
- `specs/desktop-regression-preflight-checks.md`: this implementation spec.

No CI workflow, cross-platform test harness, or suite scenario file should be
changed unless explicitly requested.

## Code Style

Use the existing PowerShell style: strict mode, explicit parameter declarations,
clear failure text, small helper functions with the `DesktopRegression` prefix,
and `pscustomobject` results that can be rendered both to console and JSON.

```powershell
function Test-DesktopRegressionShell {
    param(
        [Parameter(Mandatory = $true)][string]$Name
    )

    $command = Get-Command -Name $Name -ErrorAction SilentlyContinue
    if (-not $command) {
        return [pscustomobject]@{
            Name = "shell:$Name"
            Status = "fail"
            Message = "Shell '$Name' was not found on PATH"
        }
    }

    return [pscustomobject]@{
        Name = "shell:$Name"
        Status = "pass"
        Message = "Resolved $Name at $($command.Source)"
    }
}
```

Conventions:

- Use `pass`, `warn`, and `fail` status values consistently.
- Treat checks as data first, then render them for humans.
- Keep failures actionable: include the exact command-line option or path that
  triggered the failure.
- Do not bury preflight-only behavior inside suite files.
- Preserve current `-List` behavior and existing suite registration style.

## Testing Strategy

- Add non-invasive tests or smoke checks for `-Check` that do not move input or
  launch suites.
- Verify `-Check` exits `0` when required dependencies are present and no fatal
  preflight checks fail.
- Verify `-Check -SkipBuild -ExePath <missing>` exits non-zero and reports the
  resolved missing binary path.
- Verify `-Check -SnapShell <missing-shell>` exits non-zero and names the shell
  that could not be resolved.
- Verify `-List` still exits before Win32 initialization and does not require
  desktop access.
- Manually verify on a real Windows desktop that screen metrics, DPI awareness,
  and interactive desktop checks report useful values.
- Manually verify stale `terminal-manager` handling with a running process:
  preflight should identify the process and either stop it only when the chosen
  policy allows that, or fail with instructions to close it.

Suggested commands:

- `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -List`
- `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Check`
- `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Check -SkipBuild -ExePath missing.exe`
- `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Check -SnapShell definitely-missing-shell`

Full desktop suite execution remains manual because these suites control the
local desktop.

## Boundaries

- Always: keep preflight non-invasive by default.
- Always: validate path resolution the same way normal execution does for
  `-ExePath`, `-ArtifactsDir`, and default `target\debug\terminal-manager.exe`.
- Always: report environment details needed to reproduce a desktop failure:
  Windows version, PowerShell version, screen bounds, DPI state, shell path,
  binary path, interactive session state, and stale process state.
- Always: fail before suite execution when a required capability is missing.
- Ask first: adding new dependencies, changing runner defaults, or making
  `pwsh` the canonical command.
- Ask first: adding CI coverage for these manual desktop checks.
- Ask first: automatically killing stale processes during check-only mode.
- Never: run suite scriptblocks, move the cursor, send keys, or capture user
  content screenshots during `-Check`.
- Never: hide a failed prerequisite behind a later suite failure.
- Never: change compatibility wrapper behavior outside the desktop regression
  runner unless explicitly scoped.

## Plan

1. Define the preflight result model.
   - Create a small result shape with `Name`, `Status`, `Message`, and optional
     `Details`.
   - Decide that any `fail` exits non-zero, any `warn` exits zero in check-only
     mode unless paired with a fatal failure.

2. Add helper functions in `lib/DesktopRegression.ps1`.
   - Check Windows OS version and reject non-Windows or unsupported major
     versions.
   - Check `$PSVersionTable` against the current 5.1 requirement and report
     edition.
   - Initialize Win32 and .NET assemblies only as needed for checks that require
     them.
   - Check screen size, monitor count if practical, and DPI awareness state.
   - Check configured shell resolution using `Get-Command`.
   - Check binary path existence, leaf name, and file accessibility after
     normal runner path normalization.
   - Check interactive desktop capability using user-interactive process state
     and safe Win32 calls that do not move input.
   - Check for stale `terminal-manager` processes and apply the selected policy.

3. Wire `-Check` into `run.ps1`.
   - Add `[switch]$Check` to the param block.
   - Preserve `-List` as the earliest exit.
   - Normalize `-ExePath` and `-ArtifactsDir` before invoking preflight.
   - Run build before binary existence checks unless `-SkipBuild` is set, or
     explicitly report that the binary is expected to be produced by build.
   - In check-only mode, render the report and exit before suite selection and
     execution.
   - In normal mode, run fatal preflight checks before suites begin.

4. Document the command contract.
   - Update `SPEC.md` and `README.md` with the preflight command, report
     statuses, and stale process policy.
   - Keep docs clear that this remains a manual desktop framework, not CI.

5. Verify behavior.
   - Exercise `-List`, successful `-Check`, missing binary, missing shell, and
     stale process cases.
   - Confirm no suite artifacts are generated by failed check-only runs except
     the optional preflight report.

## Tasks

- [ ] Task: Add preflight result helpers.
  - Acceptance: `lib/DesktopRegression.ps1` can create, aggregate, render, and
    fail on `pass`, `warn`, and `fail` preflight results.
  - Verify: Run a lightweight PowerShell invocation that dot-sources the library
    and formats sample results.
  - Files: `tests/windows/desktop-regression/lib/DesktopRegression.ps1`.

- [ ] Task: Implement environment checks.
  - Acceptance: Helpers validate Windows version, PowerShell version, screen and
    DPI state, shell availability, binary path, permissions, interactive
    desktop state, and stale `terminal-manager` processes.
  - Verify: Run `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Check` on a real Windows desktop.
  - Files: `tests/windows/desktop-regression/lib/DesktopRegression.ps1`.

- [ ] Task: Wire `-Check` into the runner.
  - Acceptance: `run.ps1 -Check` performs preflight and exits before suite
    execution; normal suite runs fail fast on fatal preflight failures.
  - Verify: Run `-List`, `-Check`, `-Check -SkipBuild -ExePath missing.exe`,
    and `-Check -SnapShell definitely-missing-shell`.
  - Files: `tests/windows/desktop-regression/run.ps1`.

- [ ] Task: Define stale process policy.
  - Acceptance: Preflight reports stale `terminal-manager` processes and uses a
    documented policy for whether check-only mode may stop them.
  - Verify: Start `terminal-manager.exe`, run `-Check`, and confirm the report
    either stops or blocks with clear remediation according to the selected
    policy.
  - Files: `tests/windows/desktop-regression/lib/DesktopRegression.ps1`,
    `tests/windows/desktop-regression/run.ps1`.

- [ ] Task: Update framework documentation.
  - Acceptance: `SPEC.md` and `README.md` describe `-Check`, statuses, common
    failures, and the manual desktop boundary.
  - Verify: Read the docs and run each documented command that is non-invasive.
  - Files: `tests/windows/desktop-regression/SPEC.md`,
    `tests/windows/desktop-regression/README.md`.

## Success Criteria

- `run.ps1 -Check` exists and validates all required preflight categories before
  any desktop suite runs.
- `-List` remains a fast suite-discovery command and is not blocked by desktop
  preflight requirements.
- Fatal preflight failures produce non-zero exits and actionable messages.
- Check-only mode does not move the mouse, send keyboard input, run suite
  scriptblocks, or capture arbitrary desktop screenshots.
- Normal suite execution fails early when the machine cannot support reliable
  desktop regression testing.
- Stale `terminal-manager` processes are handled deliberately instead of being
  silently killed only inside `Start-DesktopRegressionApp`.
- Documentation gives engineers a clear command to validate a machine before
  running manual suites.

## Open Questions

- Should `-Check` automatically stop stale `terminal-manager` processes, or
  should check-only mode fail and require an explicit cleanup flag?
- What exact Windows build should be the minimum supported target: Windows 10
  generally, Windows 10 1903+, or only the versions actively used by maintainers?
- Should high-DPI or multi-monitor configurations fail, warn, or only record
  diagnostic details?
- Should a JSON preflight report always be written, or only when an artifacts
  directory already exists for a normal run?
- Should `pwsh` be supported as an equivalent runner once `System.Windows.Forms`
  and `System.Drawing` behavior is verified there?
