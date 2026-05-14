# Spec: Desktop Regression Failure Diagnostics

## Objective

Add automatic failure diagnostics to `tests/windows/desktop-regression` so a
failed suite leaves enough evidence to debug the failure without rerunning the
desktop interaction immediately.

The diagnostics should be captured by the framework when `run.ps1` catches a
suite failure. A failing run should preserve the current full-screen screenshot,
window rectangle, process status, foreground window title, relevant logs when
they are available, a structured failure manifest, and explicit artifact links
from both `results.json` and console output.

Success means an engineer can open one failed run directory under
`artifacts/windows/desktop-regression/<run_id>/`, read the manifest, and find
the screenshot and other evidence that explains what the desktop looked like
when the assertion failed.

## Assumptions

- Diagnostics are for the existing manual Windows desktop runner, not CI.
- The first implementation should not require every suite to change its
  assertion code; failure capture belongs in the shared runner/library path.
- App stdout or structured app logs may not exist today. The feature should link
  logs if they are present and explicitly report an empty log list if not.
- Artifact paths should remain deterministic and grouped under the current
  `RunArtifactsDir`.

## Tech Stack

- Windows PowerShell 5.1, matching the existing `#Requires -Version 5.1` files.
- `user32.dll` interop via `Add-Type` in
  `tests/windows/desktop-regression/lib/DesktopRegression.ps1`.
- .NET `System.Drawing` and `System.Windows.Forms`, already loaded by
  `Initialize-DesktopRegressionWin32` for screenshots and input.
- Existing Rust binary build through `cargo build` from
  `tests/windows/desktop-regression/run.ps1`.
- JSON serialization with PowerShell `ConvertTo-Json` and `Set-Content
  -Encoding UTF8`, matching the current `results.json` writer.

## Commands

- List suites without controlling the desktop:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -List`
- Run all suites and produce diagnostics on failure:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1`
- Run one suite and produce diagnostics on failure:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Suite post-resize-glitches`
- Run against an existing binary:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -SkipBuild`
- Intentionally force a manual diagnostic capture after implementation:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -SkipBuild -Suite post-resize-glitches -SnapLitRatioThreshold 1.0`

## Project Structure

- `tests/windows/desktop-regression/run.ps1`: suite discovery, build step,
  catch block, `results.json`, and console links to failure diagnostics.
- `tests/windows/desktop-regression/lib/DesktopRegression.ps1`: shared Win32
  declarations, process/window lifecycle, screenshot capture, artifact path
  helpers, and the new failure diagnostic capture helpers.
- `tests/windows/desktop-regression/suites/*.ps1`: behavior suites that should
  continue to use `Start-DesktopRegressionApp`, `Stop-DesktopRegressionApp`,
  `New-DesktopRegressionArtifactPath`, `Capture-DesktopRegressionScreen`, and
  assertion helpers.
- `tests/windows/desktop-regression/SPEC.md`: framework contract and manual test
  boundaries that diagnostics must preserve.
- `tests/windows/desktop-regression/README.md`: user-facing documentation for
  finding failure artifacts.
- `artifacts/windows/desktop-regression/<run_id>/results.json`: aggregate suite
  status with links to per-suite diagnostics.
- `artifacts/windows/desktop-regression/<run_id>/<suite>-failure-final.png`:
  final desktop screenshot captured after failure.
- `artifacts/windows/desktop-regression/<run_id>/<suite>-failure-manifest.json`:
  structured manifest for the failed suite.
- `artifacts/windows/desktop-regression/<run_id>/<suite>-*.log`: optional log
  artifacts if framework or application logs are available.

## Code Style

Use the existing PowerShell style: `Verb-DesktopRegressionNoun` helpers,
explicit parameters, `Set-StrictMode -Version Latest`, ordered JSON objects,
deterministic artifact names, and clear failure-tolerant diagnostics. Diagnostic
collection should not mask the original test failure.

```powershell
function Save-DesktopRegressionFailureDiagnostics {
    param(
        [Parameter(Mandatory = $true)]$Context,
        [Parameter(Mandatory = $true)]$Suite,
        [Parameter(Mandatory = $true)]$ErrorRecord
    )

    $screenshotPath = New-DesktopRegressionArtifactPath `
        -Context $Context `
        -SuiteName $Suite.Name `
        -Name "failure-final"

    $manifestPath = New-DesktopRegressionArtifactPath `
        -Context $Context `
        -SuiteName $Suite.Name `
        -Name "failure-manifest" `
        -Extension "json"

    $artifacts = @()
    try {
        Capture-DesktopRegressionScreen -Path $screenshotPath
        $artifacts += [ordered]@{
            kind = "screenshot"
            path = $screenshotPath
            description = "Full desktop screenshot captured after failure"
        }
    } catch {
        $artifacts += [ordered]@{
            kind = "screenshot"
            path = $screenshotPath
            capture_error = $_.Exception.Message
        }
    }

    $manifest = [ordered]@{
        schema_version = 1
        run_id = $Context.RunId
        suite = [ordered]@{
            name = $Suite.Name
            title = $Suite.Title
            covers = $Suite.Covers
        }
        failure = [ordered]@{
            message = $ErrorRecord.Exception.Message
            type = $ErrorRecord.Exception.GetType().FullName
            position = $ErrorRecord.InvocationInfo.PositionMessage
        }
        foreground_window = Get-DesktopRegressionForegroundWindowSnapshot
        active_session = Get-DesktopRegressionActiveSessionSnapshot -Context $Context
        artifacts = $artifacts
        logs = Get-DesktopRegressionRelevantLogs -Context $Context -SuiteName $Suite.Name
    }

    $manifest | ConvertTo-Json -Depth 8 | Set-Content -Path $manifestPath -Encoding UTF8
    return [pscustomobject]@{
        ManifestPath = $manifestPath
        ScreenshotPath = $screenshotPath
        Artifacts = $artifacts
    }
}
```

## Testing Strategy

- Keep `-List` as the first smoke test because it does not control the desktop
  and should remain unaffected by diagnostics.
- Add unit-style helper coverage by dot-sourcing `DesktopRegression.ps1`,
  creating a fake context and fake suite object, invoking the manifest writer
  with a synthetic error, and asserting that JSON is valid even when no active
  window or logs exist.
- Run a manual failure integration check with an intentionally impossible
  threshold, such as `-SnapLitRatioThreshold 1.0`, and confirm that the failed
  run writes `results.json`, a failure manifest, and a final screenshot.
- Confirm the diagnostics path is failure-tolerant by simulating screenshot or
  window lookup errors and verifying the original suite error is still the
  reported failure.
- Confirm existing suite artifacts from `post-resize-glitches` and
  `edge-resize-stability` still appear because suites already call
  `New-DesktopRegressionArtifactPath` and `Capture-DesktopRegressionScreen`.

## Boundaries

- Always: preserve the original exception message in `results.json`.
- Always: write diagnostics under the existing per-run artifact directory.
- Always: make diagnostic helper failures non-fatal and record their own capture
  errors in the manifest.
- Always: include artifact links as paths that can be opened directly from the
  local workspace.
- Ask first: adding new dependencies, changing screenshot file formats, or
  introducing external log collectors.
- Ask first: changing the suite authoring contract in a way that requires every
  suite to manage diagnostic state manually.
- Ask first: adding these desktop suites or diagnostic checks to CI.
- Never: hide or replace the assertion failure with a diagnostic collection
  error.
- Never: delete existing screenshots, logs, or manifests while collecting
  diagnostics.
- Never: capture secrets from unrelated processes intentionally; only include
  window/process metadata and logs already produced by the framework or target
  app.

## Plan

1. Extend Win32 helpers with foreground-window support.
   Add `GetForegroundWindow` interop and small helpers that return a safe
   snapshot with handle, title, and owning process id when available.

2. Track the active desktop app session in the context.
   `New-DesktopRegressionContext` should initialize active session fields, and
   `Start-DesktopRegressionApp` / `Stop-DesktopRegressionApp` should update
   them so `run.ps1` can collect process and window state even though suites
   keep `$session` local.

3. Add failure diagnostic capture helpers.
   Implement a single public helper that captures the final screenshot, active
   session snapshot, foreground window snapshot, optional logs, and JSON
   manifest.

4. Integrate diagnostics into the runner catch block.
   On suite failure, call the helper, add diagnostic paths to the failed result
   object, and print a concise `diagnostics=<path>` line next to the existing
   `[FAIL]` output.

5. Document artifact discovery.
   Update `README.md` and `SPEC.md` to describe the failure manifest, final
   screenshot, optional logs, and manual nature of the diagnostics.

6. Verify with smoke and manual failure checks.
   Run `-List`, then run one intentionally failing manual suite and inspect
   `results.json`, the manifest, the final screenshot, and existing suite
   screenshots.

## Tasks

- [ ] Task: Add foreground window and title helpers.
  - Acceptance: `DesktopRegression.ps1` can return the current foreground
    window handle, title, and process id without throwing when lookup fails.
  - Verify: dot-source the library after `Initialize-DesktopRegressionWin32`
    and call the helper from a PowerShell prompt.
  - Files: `tests/windows/desktop-regression/lib/DesktopRegression.ps1`

- [ ] Task: Track active app session on the context.
  - Acceptance: after `Start-DesktopRegressionApp`, the context exposes the
    process id and window handle for the app under test; after
    `Stop-DesktopRegressionApp`, stale handles are cleared or marked inactive.
  - Verify: run one suite and confirm the manifest can identify the target
    process/window on failure.
  - Files: `tests/windows/desktop-regression/lib/DesktopRegression.ps1`

- [ ] Task: Implement process, window, and log snapshot helpers.
  - Acceptance: snapshots include process id, path, `has_exited`, exit code
    when available, raw rect fields, formatted rect text, foreground title, and
    `logs = @()` when no logs are present.
  - Verify: helper tests with fake/missing process state produce valid JSON.
  - Files: `tests/windows/desktop-regression/lib/DesktopRegression.ps1`

- [ ] Task: Implement `Save-DesktopRegressionFailureDiagnostics`.
  - Acceptance: one helper writes `<suite>-failure-final.png` and
    `<suite>-failure-manifest.json`, returns artifact paths, and records
    diagnostic capture errors without throwing.
  - Verify: invoke the helper with a synthetic error and confirm both manifest
    JSON validity and screenshot behavior on a real desktop.
  - Files: `tests/windows/desktop-regression/lib/DesktopRegression.ps1`

- [ ] Task: Wire diagnostics into the runner failure path.
  - Acceptance: failed result entries include a `diagnostics` object with
    manifest, screenshot, artifact, and log links; console output prints the
    manifest path.
  - Verify: run an intentionally failing suite and inspect
    `artifacts/windows/desktop-regression/<run_id>/results.json`.
  - Files: `tests/windows/desktop-regression/run.ps1`

- [ ] Task: Update framework docs.
  - Acceptance: README and framework SPEC explain where failure diagnostics are
    written, what evidence is captured, and that full suite execution remains
    manual because it controls the desktop.
  - Verify: read the docs and confirm all commands use the canonical
    `tests\windows\desktop-regression\run.ps1` path.
  - Files: `tests/windows/desktop-regression/README.md`,
    `tests/windows/desktop-regression/SPEC.md`

## Success Criteria

- On any suite exception, the runner writes a final screenshot artifact if
  screen capture is possible.
- The failure manifest includes suite identity, original error details, active
  app process status, active window rectangle when available, foreground window
  title, optional logs, and artifact links.
- `results.json` contains diagnostic links for failed suites while preserving
  the current passed/failed result records.
- Diagnostic capture failures are represented in the manifest and do not change
  the runner's exit code semantics.
- Existing suite screenshots and stdout details from `post-resize-glitches` and
  `edge-resize-stability` remain available.
- `-List` still works without initializing diagnostic artifacts or controlling
  the desktop.

## Open Questions

- Should runner stdout/stderr be captured into per-suite log files, or should
  diagnostics only link logs that already exist?
- Should the manifest use absolute paths, run-directory-relative paths, or both?
- Should screenshots remain full-screen captures, or should diagnostics also
  crop a second image to the app window rectangle when that rectangle is known?
- Should process status include child processes once the app launches shells, or
  only the top-level `terminal-manager.exe` process?
- How many lines from optional logs should be summarized in the manifest versus
  linked as separate artifacts?
