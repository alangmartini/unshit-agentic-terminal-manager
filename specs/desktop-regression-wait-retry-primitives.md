# Spec: Desktop Regression Wait/Retry Primitives

## Objective

Add condition-driven wait and retry primitives to
`tests/windows/desktop-regression` so desktop suites depend on observable state
instead of broad fixed sleeps wherever practical.

The current framework already polls for the first native window in
`Get-DesktopRegressionProcessWindow`, but `Start-DesktopRegressionApp`,
`Focus-DesktopRegressionWindow`, `Invoke-DesktopRegressionLeftEdgeDrag`,
`Send-DesktopRegressionWinLeft`, and the existing suites still use fixed
`Start-Sleep` delays after launch, focus, resize, snap, command entry, drag, and
screen capture. Those sleeps make the manual desktop suites slower on fast
machines and flaky on slow, high-DPI, remote, or compositor-busy machines.

Introduce reusable helpers in `lib/DesktopRegression.ps1`:

- `Wait-ForWindow`: wait for a visible process window matching the product
  window title or process handle criteria.
- `Wait-ForRectChange`: wait for a window rectangle to change, match an
  expected predicate, or remain stable for consecutive samples.
- `Wait-ForStableScreenshot`: wait until screen capture output is stable across
  consecutive samples before pixel assertions read it.
- `Wait-ForForeground`: wait until the target handle becomes foreground after
  focus input.
- `Retry-Assertion`: rerun an assertion scriptblock until it passes or a
  timeout expires, preserving the last failure message.

Success means `post-resize-glitches` and `edge-resize-stability` retain their
manual desktop behavior while replacing coarse waits such as 700 ms, 800 ms,
1500 ms, and post-focus sleeps with explicit readiness checks where feasible.
Short sleeps that intentionally pace mouse and keyboard event sequences may
remain when they are part of input synthesis rather than readiness waiting.

## Tech Stack

- Windows PowerShell 5.1, matching `#Requires -Version 5.1` in the runner,
  library, and suites.
- Existing Win32 interop from `Initialize-DesktopRegressionWin32` in
  `tests/windows/desktop-regression/lib/DesktopRegression.ps1`.
- Existing .NET `System.Drawing` and `System.Windows.Forms` assemblies for
  screenshots and keyboard input.
- Built-in PowerShell and .NET primitives only:
  `[System.Diagnostics.Stopwatch]`, `Start-Sleep`, scriptblocks,
  `[System.Drawing.Bitmap]`, and existing Win32 calls.
- Existing manual runner:
  `tests/windows/desktop-regression/run.ps1`.
- Existing suites:
  `post-resize-glitches` and `edge-resize-stability`.

No external PowerShell modules or package dependencies should be added.

## Commands

- List suites without controlling the desktop:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -List`
- Run all desktop regression suites:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1`
- Run one suite after implementing wait primitives:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Suite post-resize-glitches`
- Run resize stability suite against an existing binary:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -SkipBuild -Suite edge-resize-stability`
- Smoke-load the helper library without running suites:
  `powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ". .\tests\windows\desktop-regression\lib\DesktopRegression.ps1; Initialize-DesktopRegressionWin32; 'loaded'"`
- Find remaining fixed sleeps during implementation review:
  `rg -n "Start-Sleep" tests/windows/desktop-regression`

Full suite execution remains manual because the framework moves the mouse,
sends keys, focuses windows, uses compositor snap behavior, and captures the
screen.

## Project Structure

- `tests/windows/desktop-regression/run.ps1`: runner, suite discovery,
  optional build, execution loop, result writing, and context creation. It
  should not duplicate wait logic.
- `tests/windows/desktop-regression/lib/DesktopRegression.ps1`: shared Win32,
  lifecycle, input, screenshot, pixel, assertion, and new wait/retry helpers.
- `tests/windows/desktop-regression/suites/post-resize-glitches.ps1`: replace
  fixed waits after window positioning, terminal clear, `Win+Left`, and
  screenshot capture with wait helpers where observable conditions exist.
- `tests/windows/desktop-regression/suites/edge-resize-stability.ps1`: replace
  fixed waits after window positioning and edge drags with rectangle stability
  checks where feasible.
- `tests/windows/desktop-regression/SPEC.md`: update after implementation to
  document wait/retry primitives as the preferred suite authoring approach.
- `tests/windows/desktop-regression/README.md`: update after implementation to
  mention that suites should use library waits instead of ad hoc sleeps.
- `tests/windows/desktop-regression/templates/suite.ps1`: update after
  implementation so new suites model the wait/retry style.

## Code Style

Keep the helper names short and verb-oriented, preserve strict-mode compatible
PowerShell, and make timeout failures actionable. Public helpers should use the
same style as existing functions: explicit params, clear thrown messages, and
small return objects when extra context is useful.

```powershell
function Retry-Assertion {
    param(
        [Parameter(Mandatory = $true)][scriptblock]$Assertion,
        [int]$TimeoutMs = 3000,
        [int]$IntervalMs = 100,
        [string]$Name = "assertion"
    )

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $lastError = $null

    while ($sw.ElapsedMilliseconds -le $TimeoutMs) {
        try {
            & $Assertion
            return
        } catch {
            $lastError = $_.Exception.Message
            Start-Sleep -Milliseconds $IntervalMs
        }
    }

    throw "$Name did not pass within ${TimeoutMs}ms. Last failure: $lastError"
}

function Wait-ForRectChange {
    param(
        [Parameter(Mandatory = $true)][IntPtr]$Handle,
        [Parameter(Mandatory = $true)]$InitialRect,
        [Parameter(Mandatory = $true)][scriptblock]$Predicate,
        [int]$TimeoutMs = 3000,
        [int]$IntervalMs = 50,
        [string]$Name = "window rectangle"
    )

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $lastRect = $null

    while ($sw.ElapsedMilliseconds -le $TimeoutMs) {
        $lastRect = Get-DesktopRegressionRect -Handle $Handle
        if (& $Predicate $InitialRect $lastRect) {
            return $lastRect
        }
        Start-Sleep -Milliseconds $IntervalMs
    }

    throw "$Name did not reach expected state within ${TimeoutMs}ms; last=$(
        Format-DesktopRegressionRect -Rect $lastRect)"
}
```

Conventions:

- Prefer `TimeoutMs`, `IntervalMs`, `StableSamples`, and `Name` parameters on
  all wait helpers.
- Throw timeout errors with the helper name, expected condition, timeout, and
  last observed state.
- Return the final observed window handle, rectangle, screenshot path, or
  assertion result so suites do not need to re-query immediately.
- Keep polling intervals conservative enough for manual desktop tests; default
  to 50-100 ms unless a helper is pacing synthesized input.
- Do not hide unbounded waits behind helper names. Every wait must have a
  timeout.

## Testing Strategy

- Use `-List` as the first smoke check. It should still avoid Win32
  initialization, app launch, artifact creation, and desktop control.
- Dot-source `lib/DesktopRegression.ps1` and run helper-level checks for
  timeout behavior, successful retry behavior, and failure messages without
  launching `terminal-manager.exe`.
- Manually run `post-resize-glitches` and verify that:
  - window positioning waits for the requested rectangle or stable rectangle;
  - `Win+Left` waits for the snap-induced size change instead of a fixed
    1500 ms delay;
  - screenshots used for pixel assertions are captured only after stability.
- Manually run `edge-resize-stability` and verify that:
  - initial positioning waits for rectangle stability;
  - inward and outward drag assertions wait for right-edge and left-edge
    conditions before failing.
- Use `rg -n "Start-Sleep" tests/windows/desktop-regression` after
  implementation to classify each remaining sleep as either input pacing or a
  candidate for replacement.
- Preserve artifacts and result behavior: screenshots still land under
  `artifacts/windows/desktop-regression/<run_id>/`, and `results.json` still
  reflects pass/fail status and error text.

## Boundaries

- Always: keep full desktop suite execution manual and operator-visible.
- Always: keep wait helpers in `lib/DesktopRegression.ps1`; suites should call
  helpers rather than reimplementing polling loops.
- Always: preserve existing runner commands, suite registration, artifact paths,
  exit codes, and assertion semantics.
- Always: include bounded timeouts and useful timeout diagnostics.
- Always: distinguish readiness waits from input pacing sleeps. Pacing sleeps
  inside mouse or keyboard synthesis may stay if removing them would make input
  unreliable.
- Ask first: changing the runner CLI surface, adding dependencies, changing
  pixel thresholds, or adding these manual suites to CI/CD.
- Ask first: introducing screenshot diff algorithms that materially increase
  run time or artifact volume.
- Never: create unbounded polling loops, swallow assertion failures without
  reporting the last failure, or capture screenshots outside explicit suite
  helper calls.
- Never: modify unrelated specs or other workers' files.

## Success Criteria

- `lib/DesktopRegression.ps1` exposes `Wait-ForWindow`,
  `Wait-ForRectChange`, `Wait-ForStableScreenshot`, `Wait-ForForeground`, and
  `Retry-Assertion`.
- `Start-DesktopRegressionApp` uses `Wait-ForWindow` or equivalent shared
  window wait logic instead of adding an extra fixed launch delay after the
  window handle is found.
- `Focus-DesktopRegressionWindow` verifies foreground state with
  `Wait-ForForeground` instead of relying only on post-click sleeps.
- `post-resize-glitches.ps1` no longer relies on fixed sleeps for window
  positioning, clear-command readiness, snap completion, or pre-assertion
  screenshot readiness when an observable condition is available.
- `edge-resize-stability.ps1` no longer relies on fixed sleeps for initial
  positioning or drag completion when rectangle conditions are observable.
- Remaining `Start-Sleep` calls in `tests/windows/desktop-regression` are either
  short input pacing waits or explicitly justified in code comments.
- Timeout failures point to the specific wait condition and include the last
  observed handle, rectangle, foreground handle, screenshot hash, or assertion
  message.
- Existing documented commands in `README.md` and `SPEC.md` continue to work.

## Open Questions

- Should `Wait-ForStableScreenshot` compare full screenshot hashes, sampled
  pixels in relevant regions, or both?
- Should screenshot stability use temporary in-memory bitmaps, temporary files,
  or final artifact paths with cleanup of intermediate captures?
- What default timeout should the framework use for compositor-driven waits:
  3000 ms, 5000 ms, or a context-level configurable value?
- Should `Retry-Assertion` be limited to known desktop regression assertion
  helpers, or should it accept any throwing scriptblock?
- Should wait timing metrics be written to `results.json` in a future result
  schema change, or only printed when a wait times out?
- Should helpers expose `-Quiet` or `-Verbose` output, or should they stay
  silent unless they fail?

## Plan

1. Inventory and classify current waits.
   - Use `rg -n "Start-Sleep" tests/windows/desktop-regression` to separate
     readiness waits from input pacing waits.
   - Treat app launch, focus completion, rectangle changes, snap completion,
     and pre-assertion screenshot readiness as replacement candidates.

2. Add shared bounded wait primitives.
   - Implement a small polling core or consistent loop pattern in
     `lib/DesktopRegression.ps1`.
   - Add `Wait-ForWindow`, `Wait-ForRectChange`,
     `Wait-ForStableScreenshot`, `Wait-ForForeground`, and
     `Retry-Assertion`.
   - Ensure every timeout includes the last observed state.

3. Replace helper-level readiness sleeps.
   - Update app launch to use `Wait-ForWindow`.
   - Update focus handling to call `Wait-ForForeground`.
   - Update rectangle-changing helpers to return only after the resulting
     rectangle is stable when feasible.

4. Replace suite-level readiness sleeps.
   - Update `post-resize-glitches` around initial resize, shell clearing,
     `Win+Left`, and screenshot timing.
   - Update `edge-resize-stability` around initial resize and drag completion.
   - Keep short sleeps that pace mouse down/up, key down/up, and drag stepping.

5. Document and verify.
   - Update `README.md`, `SPEC.md`, and the suite template after
     implementation.
   - Run non-invasive smoke checks and manual desktop suites on a real Windows
     desktop.

## Tasks

- [ ] Task: Classify existing sleeps.
  - Acceptance: every current `Start-Sleep` in
    `tests/windows/desktop-regression` is categorized as readiness waiting,
    input pacing, or intentionally retained delay.
  - Verify: `rg -n "Start-Sleep" tests/windows/desktop-regression`
    plus a short implementation note or code comments for retained sleeps.
  - Files: `tests/windows/desktop-regression/lib/DesktopRegression.ps1`,
    `tests/windows/desktop-regression/suites/*.ps1`.

- [ ] Task: Add wait and retry helpers.
  - Acceptance: `Wait-ForWindow`, `Wait-ForRectChange`,
    `Wait-ForStableScreenshot`, `Wait-ForForeground`, and
    `Retry-Assertion` exist in `lib/DesktopRegression.ps1` with bounded
    timeouts and clear timeout errors.
  - Verify:
    `powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ". .\tests\windows\desktop-regression\lib\DesktopRegression.ps1; Initialize-DesktopRegressionWin32; Get-Command Wait-ForWindow,Wait-ForRectChange,Wait-ForStableScreenshot,Wait-ForForeground,Retry-Assertion"`
  - Files: `tests/windows/desktop-regression/lib/DesktopRegression.ps1`.

- [ ] Task: Convert app launch and focus helpers.
  - Acceptance: `Start-DesktopRegressionApp` no longer adds a fixed launch
    delay after a window is found, and `Focus-DesktopRegressionWindow` waits for
    the target handle to become foreground before returning.
  - Verify: run
    `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -SkipBuild -Suite edge-resize-stability`
    on a real Windows desktop and confirm app launch/focus succeeds.
  - Files: `tests/windows/desktop-regression/lib/DesktopRegression.ps1`.

- [ ] Task: Convert rectangle and screenshot readiness in suites.
  - Acceptance: `post-resize-glitches` and `edge-resize-stability` use
    rectangle waits and stable screenshot waits instead of coarse post-action
    sleeps where observable conditions exist.
  - Verify: run each suite individually and inspect artifacts for expected
    screenshots and pass/fail behavior.
  - Files:
    `tests/windows/desktop-regression/suites/post-resize-glitches.ps1`,
    `tests/windows/desktop-regression/suites/edge-resize-stability.ps1`.

- [ ] Task: Add retry around transient desktop assertions.
  - Acceptance: assertions that depend on compositor completion or paint
    settling can use `Retry-Assertion` with short intervals while preserving the
    final assertion message on timeout.
  - Verify: force or simulate a delayed passing assertion in a local helper
    check, then confirm the helper returns after the assertion passes and
    throws the last failure when it never passes.
  - Files: `tests/windows/desktop-regression/lib/DesktopRegression.ps1`,
    `tests/windows/desktop-regression/suites/*.ps1`.

- [ ] Task: Update framework documentation and template.
  - Acceptance: `SPEC.md`, `README.md`, and `templates/suite.ps1` describe wait
    helpers as the preferred pattern for new suites and avoid modeling broad
    fixed sleeps for readiness.
  - Verify: run `-List` and review docs for command accuracy.
  - Files: `tests/windows/desktop-regression/SPEC.md`,
    `tests/windows/desktop-regression/README.md`,
    `tests/windows/desktop-regression/templates/suite.ps1`.
