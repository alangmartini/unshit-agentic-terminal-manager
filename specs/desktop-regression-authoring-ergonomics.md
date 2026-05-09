# Spec: Desktop Regression Authoring Ergonomics

## Objective

Make `tests/windows/desktop-regression` suites easier to author by adding
higher-level PowerShell fixtures, assertions, and a richer suite template that
remove repeated lifecycle, artifact, rectangle, screenshot, and pixel-sampling
boilerplate.

Current suites manually call `Start-DesktopRegressionApp`, wrap every scenario
in `try/finally`, cache `$session.WindowHandle`, build screenshot paths with
`New-DesktopRegressionArtifactPath`, capture before/after evidence, format
rectangles, and inline behavior-specific assertions. That repetition makes new
suites slower to write and easier to get subtly wrong.

The implementation should introduce suite-facing helpers such as:

- `Invoke-DesktopRegressionSession`: launch the app, expose the session/window,
  run a scenario scriptblock, and always stop the app.
- `Capture-BeforeAfter`: capture labeled before/after screenshots and
  rectangles around an interaction.
- `Assert-RectStable`: assert one or more rectangle edges or dimensions stayed
  within tolerance.
- `Assert-WindowGrew`: assert a window grew along one or both dimensions with a
  useful before/after failure message.
- `Assert-NoVisualStripe`: hide the bitmap loading, stripe sampling, disposal,
  threshold comparison, and diagnostic output currently duplicated by visual
  suites.
- A richer `templates/suite.ps1` that models the preferred flow for new suites:
  metadata, session fixture, arrangement, before/after capture, assertion
  helpers, and explicit artifact labels.

Success means new desktop suites can express the protected behavior directly
while preserving the existing manual runner, suite registration, artifact
location, and command-line surface.

## Assumptions

- The helper names above are the desired public suite-authoring API, even
  though the existing lower-level helpers mostly use the
  `*-DesktopRegression*` naming pattern.
- The first implementation should be additive. Existing helpers and suites keep
  working while suites can migrate to the ergonomic helpers incrementally.
- The helpers should live in `lib/DesktopRegression.ps1`; no separate module or
  external PowerShell dependency is needed.
- The richer template should remain a PowerShell script that can be copied into
  `suites/<suite-name>.ps1`, matching the current workflow in `README.md`.

## Tech Stack

- Windows PowerShell 5.1, matching `#Requires -Version 5.1` in `run.ps1`,
  `lib/DesktopRegression.ps1`, the template, and current suites.
- Existing desktop regression runner:
  `tests/windows/desktop-regression/run.ps1`.
- Existing suite registration API:
  `Register-DesktopRegressionSuite -Name -Title -Covers -Tags -ScriptBlock`.
- Existing shared library:
  `tests/windows/desktop-regression/lib/DesktopRegression.ps1`.
- Existing Win32 interop, `System.Drawing`, and `System.Windows.Forms`
  initialization from `Initialize-DesktopRegressionWin32`.
- Existing manual artifact layout:
  `artifacts/windows/desktop-regression/<run_id>/`.
- Built-in PowerShell and .NET only. Do not add PowerShell modules, packages,
  or test dependencies for this ergonomics layer.

## Commands

- List suites without launching the app:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -List`
- Run all desktop regression suites:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1`
- Run one existing suite after helper migration:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Suite edge-resize-stability`
- Run visual suite against an existing binary:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -SkipBuild -Suite post-resize-glitches`
- Smoke-load the library and verify new helper commands exist:
  `powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ". .\tests\windows\desktop-regression\lib\DesktopRegression.ps1; Get-Command Invoke-DesktopRegressionSession,Capture-BeforeAfter,Assert-RectStable,Assert-WindowGrew,Assert-NoVisualStripe"`
- Review remaining suite boilerplate:
  `rg -n "Start-DesktopRegressionApp|Stop-DesktopRegressionApp|New-DesktopRegressionArtifactPath|Capture-DesktopRegressionScreen|FromFile" tests/windows/desktop-regression/suites tests/windows/desktop-regression/templates`

Full suite execution remains manual because the framework controls a real
Windows desktop, launches `terminal-manager.exe`, moves the mouse, sends global
input such as `Win+Left`, and captures the screen.

## Project Structure

- `tests/windows/desktop-regression/run.ps1`: keep runner behavior unchanged.
  It should continue to discover suites, create context, run selected
  scriptblocks, collect results, and write `results.json`.
- `tests/windows/desktop-regression/lib/DesktopRegression.ps1`: add the
  ergonomic session, capture, rectangle assertion, window-growth assertion, and
  visual-stripe assertion helpers here.
- `tests/windows/desktop-regression/templates/suite.ps1`: update after helper
  implementation so new suites start from the higher-level fixture pattern
  instead of manual `try/finally` and screenshot path boilerplate.
- `tests/windows/desktop-regression/suites/edge-resize-stability.ps1`: migrate
  as the primary non-visual example because it repeats lifecycle, artifact
  naming, before/after/restore capture, and rectangle assertions.
- `tests/windows/desktop-regression/suites/post-resize-glitches.ps1`: migrate
  as the primary visual example because it repeats lifecycle, before/post
  screenshots, window-growth assertion logic, bitmap loading/disposal, and
  stripe threshold assertions.
- `tests/windows/desktop-regression/README.md`: document the preferred helper
  pattern in "Adding A Suite" after implementation.
- `tests/windows/desktop-regression/SPEC.md`: document the ergonomic helper
  layer as the supported suite-authoring surface after implementation.

## Code Style

Keep helpers strict-mode compatible, explicit about parameters, and close to the
existing PowerShell style in `DesktopRegression.ps1`: verb-oriented functions,
clear thrown messages, small `[pscustomobject]` return values, and explicit
artifact labels.

```powershell
function Invoke-DesktopRegressionSession {
    param(
        [Parameter(Mandatory = $true)]$Context,
        [Parameter(Mandatory = $true)][string]$SuiteName,
        [Parameter(Mandatory = $true)][scriptblock]$Body,
        [switch]$Focus
    )

    $session = Start-DesktopRegressionApp -Context $Context
    try {
        if ($Focus) {
            Focus-DesktopRegressionWindow -Handle $session.WindowHandle
        }

        & $Body $session $session.WindowHandle $Context
    } finally {
        Stop-DesktopRegressionApp -Session $session
    }
}

function Capture-BeforeAfter {
    param(
        [Parameter(Mandatory = $true)]$Context,
        [Parameter(Mandatory = $true)][string]$SuiteName,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][IntPtr]$Handle,
        [Parameter(Mandatory = $true)][scriptblock]$Action
    )

    $beforePath = New-DesktopRegressionArtifactPath `
        -Context $Context `
        -SuiteName $SuiteName `
        -Name "$Name-before"
    $beforeRect = Get-DesktopRegressionRect -Handle $Handle
    Capture-DesktopRegressionScreen -Path $beforePath

    & $Action

    $afterPath = New-DesktopRegressionArtifactPath `
        -Context $Context `
        -SuiteName $SuiteName `
        -Name "$Name-after"
    $afterRect = Get-DesktopRegressionRect -Handle $Handle
    Capture-DesktopRegressionScreen -Path $afterPath

    return [pscustomobject]@{
        BeforeRect = $beforeRect
        AfterRect = $afterRect
        BeforePath = $beforePath
        AfterPath = $afterPath
    }
}
```

Suite usage should read like the scenario rather than the framework plumbing:

```powershell
Register-DesktopRegressionSuite `
    -Name "example-suite-name" `
    -Title "Human readable suite title" `
    -Covers "One sentence describing the desktop behavior this suite protects." `
    -Tags @("windows", "resize") `
    -ScriptBlock {
        param($Context)

        Invoke-DesktopRegressionSession -Context $Context -SuiteName "example-suite-name" -Focus -Body {
            param($Session, $Handle, $Context)

            $capture = Capture-BeforeAfter `
                -Context $Context `
                -SuiteName "example-suite-name" `
                -Name "left-edge-drag" `
                -Handle $Handle `
                -Action {
                    Invoke-DesktopRegressionLeftEdgeDrag `
                        -Handle $Handle `
                        -FromY 400 `
                        -FromX 4 `
                        -ToX 224
                }

            Assert-RectStable `
                -Before $capture.BeforeRect `
                -After $capture.AfterRect `
                -Edges @("Right") `
                -Tolerance $Context.Tolerance `
                -Name "right edge during left-edge drag"
        }
    }
```

Conventions:

- Prefer helper parameters named `Context`, `SuiteName`, `Name`, `Handle`,
  `Before`, `After`, `Tolerance`, and `Threshold` to match current suite
  vocabulary.
- Return captured paths and rectangles so suites can print or assert without
  re-querying.
- Include before and after rectangle formatting in failure messages for
  geometry assertions.
- Dispose `System.Drawing.Bitmap` values inside helpers that load screenshots.
- Keep lower-level helpers available for unusual desktop interactions.

## Testing Strategy

- Start with `-List` to confirm suite discovery and template dot-sourcing still
  work without launching the app.
- Dot-source `lib/DesktopRegression.ps1` and verify the new commands are
  discoverable with `Get-Command`.
- Add helper-level checks where possible without controlling the desktop:
  rectangle comparison helpers can be exercised with lightweight mock rectangle
  objects, and threshold helpers can be exercised with small in-memory bitmaps
  or known screenshot fixtures if fixtures are introduced later.
- Manually run `edge-resize-stability` after migration and confirm:
  lifecycle is handled by `Invoke-DesktopRegressionSession`;
  before/after/restore captures still land under the run artifact directory;
  right-edge and left-edge assertions produce at least as much diagnostic
  detail as the current inline assertions.
- Manually run `post-resize-glitches` after migration and confirm:
  window growth is asserted through `Assert-WindowGrew`;
  bottom statusbar and mid-pane stripe checks are expressed through
  `Assert-NoVisualStripe` or a complementary stripe helper;
  bitmap disposal remains reliable on repeated runs.
- Use the boilerplate review command from this spec to ensure new suites and
  the template no longer model direct lifecycle/artifact plumbing except where
  a suite has a clear reason to drop to lower-level helpers.
- Preserve artifact and result behavior: screenshots remain PNG files under
  `artifacts/windows/desktop-regression/<run_id>/`, and `results.json` still
  reports pass/fail status and error text.

## Boundaries

- Always: keep the current runner CLI, suite registration API, manual execution
  model, artifact root, and result-writing behavior compatible.
- Always: implement the ergonomic helpers in `lib/DesktopRegression.ps1` so
  suites and templates share one supported authoring surface.
- Always: make new helpers additive until existing suites have been migrated and
  reviewed.
- Always: keep lower-level helpers available for scenarios that need custom
  Win32 input, screenshots, or pixel sampling.
- Always: include suite names and artifact labels in generated screenshot paths
  and assertion messages.
- Ask first: renaming existing helpers, changing the `Register-DesktopRegressionSuite`
  contract, changing `results.json`, adding dependencies, or moving these
  manual suites into CI/CD.
- Ask first: adding a full PowerShell test framework such as Pester.
- Never: hide app cleanup behind code paths that can skip
  `Stop-DesktopRegressionApp`.
- Never: capture screenshots outside explicit suite actions or helper calls.
- Never: delete or weaken existing behavior assertions during migration.
- Never: modify unrelated specs or other workers' files.

## Success Criteria

- `lib/DesktopRegression.ps1` exposes `Invoke-DesktopRegressionSession`,
  `Capture-BeforeAfter`, `Assert-RectStable`, `Assert-WindowGrew`, and
  `Assert-NoVisualStripe`.
- `Invoke-DesktopRegressionSession` starts the app, optionally focuses the
  window, invokes a scenario scriptblock with session/window/context arguments,
  and always stops the app in `finally`.
- `Capture-BeforeAfter` creates deterministic artifact paths, captures before
  and after screenshots, records before and after rectangles, and returns a
  structured object to the suite.
- `Assert-RectStable` can compare named edges or dimensions within tolerance and
  reports both formatted rectangles on failure.
- `Assert-WindowGrew` can assert width, height, or both grew by an expected
  amount or simply grew beyond tolerance.
- `Assert-NoVisualStripe` encapsulates screenshot bitmap loading, sampling,
  disposal, threshold comparison, and failure diagnostics for stale visual
  stripe checks.
- `templates/suite.ps1` demonstrates the new fixture/capture/assertion pattern
  and no longer encourages manual app lifecycle or artifact boilerplate for the
  common case.
- At least `edge-resize-stability` and `post-resize-glitches` can be migrated to
  the new helpers without changing their protected behavior, suite names, tags,
  artifact location, or documented commands.
- The `README.md` "Adding A Suite" section and `SPEC.md` framework contract
  describe the ergonomic helper layer after implementation.

## Open Questions

- Should the public helper names remain exactly
  `Invoke-DesktopRegressionSession`, `Capture-BeforeAfter`,
  `Assert-RectStable`, `Assert-WindowGrew`, and `Assert-NoVisualStripe`, or
  should they receive a `DesktopRegression` noun for consistency with existing
  functions?
- Should `Capture-BeforeAfter` support an optional third capture such as
  `restore`, or should that be a separate helper to keep the common API simple?
- Should `Assert-NoVisualStripe` accept raw rectangle coordinates, a named
  sample region object, or both?
- Should visual stripe helpers print `Write-Output` diagnostics automatically,
  or should suites decide what gets printed?
- Should `Invoke-DesktopRegressionSession` stop stale apps before every suite by
  relying on `Start-DesktopRegressionApp`, or should it expose an explicit
  `-StopStaleApps` switch for unusual debugging sessions?
- Should helper-level tests use only ad hoc PowerShell smoke commands, or is a
  future Pester dependency worth considering after the framework grows?

## Plan

1. Define the suite-authoring helper contracts.
   - Specify parameter names, return objects, failure messages, and default
     behavior for session, capture, rectangle, growth, and stripe helpers.
   - Keep the first contract additive so current suites and lower-level helpers
     remain usable.

2. Implement helper primitives in `lib/DesktopRegression.ps1`.
   - Add `Invoke-DesktopRegressionSession` around existing start/focus/stop
     helpers.
   - Add `Capture-BeforeAfter` around existing artifact, rectangle, and
     screenshot helpers.
   - Add geometry assertion helpers using `Assert-DesktopRegressionClose` and
     `Assert-DesktopRegressionTrue` where practical.
   - Add visual stripe assertion helpers around existing stripe sampling
     functions.

3. Migrate existing suites incrementally.
   - Convert `edge-resize-stability` first because it is mostly lifecycle,
     artifact, capture, and rectangle assertion boilerplate.
   - Convert `post-resize-glitches` next because it validates the visual helper
     API and bitmap disposal behavior.
   - Preserve suite names, metadata, behavior, screenshots, and console
     diagnostics.

4. Update authoring documentation.
   - Rewrite `templates/suite.ps1` to model the new common path.
   - Update `README.md` and `SPEC.md` so future suites start with fixtures and
     drop to lower-level helpers only when needed.

5. Verify manually on Windows.
   - Run non-invasive discovery and command checks first.
   - Run migrated suites one at a time on a real desktop and inspect artifacts
     and failures.

## Tasks

- [ ] Task: Finalize helper API contract.
  - Acceptance: parameter names, return shapes, default behavior, and failure
    message requirements are documented for `Invoke-DesktopRegressionSession`,
    `Capture-BeforeAfter`, `Assert-RectStable`, `Assert-WindowGrew`, and
    `Assert-NoVisualStripe`.
  - Verify: review the helper contract against the current template and both
    existing suites to confirm each repeated lifecycle/artifact/assertion block
    has an intended replacement.
  - Files: `tests/windows/desktop-regression/SPEC.md`,
    `tests/windows/desktop-regression/README.md`.

- [ ] Task: Add session and capture helpers.
  - Acceptance: `Invoke-DesktopRegressionSession` handles start/focus/body/stop
    cleanup, and `Capture-BeforeAfter` returns before/after paths and
    rectangles using existing artifact and screenshot helpers.
  - Verify:
    `powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ". .\tests\windows\desktop-regression\lib\DesktopRegression.ps1; Get-Command Invoke-DesktopRegressionSession,Capture-BeforeAfter"`
  - Files: `tests/windows/desktop-regression/lib/DesktopRegression.ps1`.

- [ ] Task: Add rectangle and growth assertions.
  - Acceptance: `Assert-RectStable` supports named edges or dimensions within
    tolerance, and `Assert-WindowGrew` reports before/after dimensions when
    growth is missing or below threshold.
  - Verify: dot-source the library and run assertion helpers against mock
    rectangle objects for passing and failing cases.
  - Files: `tests/windows/desktop-regression/lib/DesktopRegression.ps1`.

- [ ] Task: Add visual stripe assertion helper.
  - Acceptance: `Assert-NoVisualStripe` loads a screenshot, samples a requested
    stripe region with existing pixel helpers, disposes the bitmap, compares
    against a threshold, and reports label, sample coordinates, ratio, and
    threshold on failure.
  - Verify: run a helper-level check with a small generated bitmap or a captured
    screenshot artifact, then confirm the bitmap file can be deleted
    immediately after the helper returns.
  - Files: `tests/windows/desktop-regression/lib/DesktopRegression.ps1`.

- [ ] Task: Migrate existing suites to ergonomic helpers.
  - Acceptance: `edge-resize-stability` and `post-resize-glitches` use the new
    session/capture/assertion helpers for their common lifecycle, artifact,
    rectangle, growth, and stripe checks while preserving behavior.
  - Verify:
    `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Suite edge-resize-stability`
    and
    `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Suite post-resize-glitches`
    on a real Windows desktop.
  - Files:
    `tests/windows/desktop-regression/suites/edge-resize-stability.ps1`,
    `tests/windows/desktop-regression/suites/post-resize-glitches.ps1`.

- [ ] Task: Update suite template and documentation.
  - Acceptance: `templates/suite.ps1` uses `Invoke-DesktopRegressionSession`,
    `Capture-BeforeAfter`, and assertion helpers in the default example, while
    `README.md` and `SPEC.md` describe these as the preferred suite-authoring
    path.
  - Verify:
    `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -List`
    and manual review of the "Adding A Suite" instructions.
  - Files: `tests/windows/desktop-regression/templates/suite.ps1`,
    `tests/windows/desktop-regression/README.md`,
    `tests/windows/desktop-regression/SPEC.md`.

- [ ] Task: Review remaining boilerplate.
  - Acceptance: direct calls to app lifecycle, artifact path creation, raw
    screenshot capture, and bitmap loading in suites are either removed or
    intentionally retained for custom behavior that the ergonomic helpers do not
    cover.
  - Verify:
    `rg -n "Start-DesktopRegressionApp|Stop-DesktopRegressionApp|New-DesktopRegressionArtifactPath|Capture-DesktopRegressionScreen|FromFile" tests/windows/desktop-regression/suites tests/windows/desktop-regression/templates`
  - Files: `tests/windows/desktop-regression/suites/*.ps1`,
    `tests/windows/desktop-regression/templates/suite.ps1`.
