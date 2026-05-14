# Spec: Desktop Regression Framework Self-Tests

## Objective

Add a small self-test layer for the Windows Desktop Interaction Regression framework itself. These tests should protect framework behavior that can be validated without launching `terminal-manager.exe`, moving the mouse, sending global keys, taking screenshots, or otherwise controlling the desktop.

The intended users are engineers and agents changing `tests/windows/desktop-regression/run.ps1`, `tests/windows/desktop-regression/lib/DesktopRegression.ps1`, or the legacy wrapper scripts. Success means framework regressions are caught by fast PowerShell tests while the existing full desktop suites remain explicit manual checks.

The self-tests should cover:

- Suite discovery from `tests/windows/desktop-regression/suites/*.ps1`, including deterministic name ordering from the runner.
- Duplicate registration behavior from `Register-DesktopRegressionSuite`.
- Unknown `-Suite` errors from the canonical runner.
- Artifact path generation from `New-DesktopRegressionArtifactPath`, including safe file names and the run artifact root.
- Result JSON shape written by the runner for synthetic pass/fail suites.
- Compatibility wrapper forwarding for `scripts/desktop-regression/run.ps1` and legacy option mapping in `scripts/window-resize-automation.ps1`.

## Tech Stack

- PowerShell 5.1 compatible scripts, matching the `#Requires -Version 5.1` headers in the framework.
- No new module dependency unless approved; prefer plain PowerShell assertions so the tests run on a stock Windows developer machine.
- Temporary filesystem fixtures for suite files and artifacts.
- No desktop control APIs in self-test execution: no `System.Windows.Forms.SendKeys`, process launch, screenshot capture, mouse movement, or global input. If a current runner path initializes Win32 before a pure validation error, move that validation earlier or expose an internal test seam before adding the self-test.

## Commands

- Run the self-tests:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\self-tests\run.ps1`
- Run a single self-test file:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\self-tests\run.ps1 -Test framework-registration`
- List self-tests:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\self-tests\run.ps1 -List`
- Keep manual desktop suite listing available:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -List`
- Keep full desktop suites manual:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -SkipBuild`

## Project Structure

- `tests/windows/desktop-regression/run.ps1`: canonical desktop suite runner. Self-tests may exercise non-invasive paths such as `-List`, unknown suite validation, synthetic suite execution with mocked fixture suites, and `results.json` writing.
- `tests/windows/desktop-regression/lib/DesktopRegression.ps1`: registry, context, artifact path, and desktop helper library. Self-tests should directly dot-source this file for pure helpers, but must not call functions that initialize Win32 or control the desktop.
- `tests/windows/desktop-regression/self-tests/run.ps1`: proposed self-test runner for framework tests.
- `tests/windows/desktop-regression/self-tests/*.ps1`: proposed focused self-test files.
- `tests/windows/desktop-regression/self-tests/fixtures/`: proposed temporary or committed fixture suite files if fixture generation inside `$env:TEMP` is not sufficient.
- `tests/windows/desktop-regression/SPEC.md`: framework contract that continues to mark full desktop interaction as manual.
- `tests/windows/desktop-regression/README.md`: user-facing commands. Update after implementation to document the self-tests separately from manual desktop suites.
- `scripts/desktop-regression/run.ps1`: compatibility wrapper that forwards bound parameters to the canonical runner.
- `scripts/window-resize-automation.ps1`: compatibility wrapper that maps legacy snap switches to canonical suite names.
- `artifacts/windows/desktop-regression/<run_id>/`: production artifact location that self-tests should validate with temporary artifact roots, not by polluting real manual artifacts.

## Code Style

Use strict PowerShell, small assertion helpers, literal paths, and temporary directories that are cleaned up in `finally`. Keep self-test names behavior-oriented and keep tests focused on one framework contract at a time.

```powershell
#Requires -Version 5.1

[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Assert-Equal {
    param(
        [Parameter(Mandatory = $true)]$Actual,
        [Parameter(Mandatory = $true)]$Expected,
        [Parameter(Mandatory = $true)][string]$Message
    )

    if ($Actual -ne $Expected) {
        throw "$Message. Expected '$Expected', got '$Actual'."
    }
}

$root = Join-Path $env:TEMP ("desktop-regression-self-tests-{0}" -f ([guid]::NewGuid()))
New-Item -ItemType Directory -Path $root | Out-Null

try {
    . (Join-Path $PSScriptRoot "..\lib\DesktopRegression.ps1")

    Register-DesktopRegressionSuite `
        -Name "example" `
        -Title "Example suite" `
        -Covers "self-test fixture" `
        -ScriptBlock { param($Context) [void]$Context }

    $suite = Get-DesktopRegressionSuites | Select-Object -First 1
    Assert-Equal -Actual $suite.Name -Expected "example" -Message "Suite registration should preserve the suite name"
} finally {
    Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
}
```

Key conventions:

- Use `Assert-*` helpers instead of framework desktop assertions when testing the framework itself.
- Use `-LiteralPath` for cleanup and file checks.
- Use synthetic suites that only inspect context or throw controlled exceptions.
- Prefer process-level runner invocation for behavior that depends on `run.ps1` parameter binding or exit codes.
- Do not depend on the current real suite names except where explicitly testing wrapper legacy mappings.

## Testing Strategy

Implement the self-tests as PowerShell integration tests around the framework scripts, split by risk area:

- Registry tests dot-source `lib/DesktopRegression.ps1` in a fresh PowerShell process and verify successful registration, returned object shape, and duplicate-name failure text.
- Discovery tests run the canonical runner against an isolated fixture suite directory or an approved test seam, then verify suite ordering and `-List` output without calling `Initialize-DesktopRegressionWin32`.
- Unknown suite tests validate `tests/windows/desktop-regression/run.ps1 -Suite definitely-missing -SkipBuild` without desktop control. If the current runner order initializes Win32 before reporting the unknown suite, first move suite selection validation ahead of `Initialize-DesktopRegressionWin32` and cover that order with the self-test.
- Artifact path tests call `New-DesktopRegressionContext` with a temporary artifact root and verify `RunId`, `RunArtifactsDir`, extension handling, and name sanitization from `New-DesktopRegressionArtifactPath`.
- Result JSON tests execute synthetic pass and fail suites that do not touch the desktop, then verify `results.json` is valid JSON with `name`, `title`, `status`, `duration_seconds`, and `error` only on failed entries.
- Wrapper tests invoke the two scripts under `scripts/` with non-invasive options or a stub canonical runner seam, then verify forwarded parameters, suite mappings, and exit code propagation.

Self-tests should be safe to run during normal development. They should not be added to CI until the project explicitly decides Windows PowerShell self-tests are part of the automated gate.

## Boundaries

- Always: keep self-tests non-invasive, run in Windows PowerShell 5.1, clean temporary files, assert exit codes and JSON shape, and preserve the manual nature of full desktop suites.
- Always: isolate fixture suites from the real `suites/*.ps1` directory unless the test is intentionally validating current real discovery with `-List`.
- Ask first: adding Pester or any other test framework dependency, adding CI workflow steps, changing public runner parameters, or adding new command-line seams to production scripts.
- Ask first: changing wrapper compatibility behavior for `-OnlySnapTest`, `-SkipSnapTest`, `-Suite`, `-List`, or artifact parameters.
- Never: run full desktop suites from the self-test runner, send keys, move the mouse, capture the screen, start `terminal-manager.exe`, stop unrelated local processes, or overwrite real manual artifacts.

## Success Criteria

- `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\self-tests\run.ps1` completes without controlling the desktop.
- Duplicate suite registration fails with a clear error containing the duplicate suite name.
- Unknown canonical runner suite selection returns a non-zero exit and names the unknown suite plus known suites.
- Artifact paths are rooted under a temporary `windows\desktop-regression\<run_id>` directory and sanitize unsafe artifact names.
- Synthetic runner results write valid JSON with stable fields for passed and failed suites.
- Compatibility wrappers are tested for parameter forwarding, legacy suite selection, and exit code propagation.
- Documentation continues to distinguish self-tests from manual desktop interaction suites.

## Plan

1. Add a self-test runner under `tests/windows/desktop-regression/self-tests/run.ps1` that discovers self-test files, supports `-List` and `-Test`, reports pass/fail, and returns a non-zero exit when any self-test fails.
2. Add pure library self-tests for registration, duplicate registration, context creation, and artifact path generation by dot-sourcing `lib/DesktopRegression.ps1` in isolated PowerShell processes.
3. Add canonical runner self-tests using synthetic suites and temporary artifact roots. If the current runner cannot be pointed at fixture suites without touching real suites, introduce the smallest internal test seam and document it as unsupported for normal users.
4. Add wrapper compatibility self-tests for `scripts/desktop-regression/run.ps1` and `scripts/window-resize-automation.ps1`, preferably through a stub runner seam so tests do not execute real desktop suites.
5. Update `tests/windows/desktop-regression/README.md` and `SPEC.md` to show the self-test command separately from manual suite commands.

## Tasks

- [ ] Task: Create the self-test runner.
  - Acceptance: The runner lists, filters, executes self-test files, prints stable pass/fail output, and exits non-zero on failure.
  - Verify: `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\self-tests\run.ps1 -List`
  - Files: `tests/windows/desktop-regression/self-tests/run.ps1`

- [ ] Task: Add registry and duplicate registration tests.
  - Acceptance: Tests verify successful registration shape and duplicate-name failure without initializing Win32.
  - Verify: `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\self-tests\run.ps1 -Test framework-registration`
  - Files: `tests/windows/desktop-regression/self-tests/framework-registration.ps1`

- [ ] Task: Add context and artifact path tests.
  - Acceptance: Tests verify temporary artifact root creation, run id format, extension handling, and unsafe artifact name sanitization.
  - Verify: `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\self-tests\run.ps1 -Test artifact-paths`
  - Files: `tests/windows/desktop-regression/self-tests/artifact-paths.ps1`

- [ ] Task: Add canonical runner behavior tests.
  - Acceptance: Tests cover discovery ordering, unknown suite errors, synthetic pass/fail execution, exit codes, and `results.json` shape without launching the app.
  - Verify: `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\self-tests\run.ps1 -Test runner-contract`
  - Files: `tests/windows/desktop-regression/self-tests/runner-contract.ps1`, plus `tests/windows/desktop-regression/run.ps1` only if a test seam is approved.

- [ ] Task: Add compatibility wrapper tests.
  - Acceptance: Tests cover forwarding from `scripts/desktop-regression/run.ps1`, legacy suite mapping from `scripts/window-resize-automation.ps1`, and exit code propagation.
  - Verify: `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\self-tests\run.ps1 -Test compatibility-wrappers`
  - Files: `tests/windows/desktop-regression/self-tests/compatibility-wrappers.ps1`, plus wrapper scripts only if a test seam is approved.

- [ ] Task: Document the self-test workflow.
  - Acceptance: README and framework spec clearly separate safe self-tests from manual desktop suites.
  - Verify: `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\self-tests\run.ps1`
  - Files: `tests/windows/desktop-regression/README.md`, `tests/windows/desktop-regression/SPEC.md`

## Open Questions

- Should the canonical runner gain an internal-only fixture suite directory parameter, or should self-tests invoke a copied runner fixture to avoid changing the public command surface?
- Should these self-tests remain developer-only, or should a later Windows CI job run only the non-invasive self-test runner?
- Is a no-dependency assertion runner preferred long term, or is Pester acceptable if more PowerShell tests are added elsewhere?
- Should compatibility wrapper tests use a stub runner seam, or is invoking wrapper `-List` against real suites sufficient coverage for now?
