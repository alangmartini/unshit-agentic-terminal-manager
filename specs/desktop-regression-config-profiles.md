# Spec: Desktop Regression Config Profiles

## Objective

Add named configuration profiles to
`tests/windows/desktop-regression/run.ps1` so common Desktop Interaction
Regression modes can be selected with one option instead of repeating the long
flat runner parameter list.

The feature should serve engineers and agents running the manual Windows
desktop regression framework documented in
`tests/windows/desktop-regression/SPEC.md` and
`tests/windows/desktop-regression/README.md`. Profiles must make the current
defaults explicit, support stricter and more forgiving local runs, provide a
short smoke profile for supervised automation, and provide a snap-focused
debugging profile.

The initial named profiles are:

- `default`: current behavior and current parameter defaults.
- `strict`: full suites with tighter geometry and visual thresholds.
- `relaxed`: full suites with more forgiving thresholds for noisy desktop
  environments.
- `ci-smoke`: a short, deterministic subset suitable for a headed Windows
  smoke run, without adding the framework to CI/CD.
- `snap-debug`: snap-focused settings that run `post-resize-glitches` and make
  snap visual parameters easy to inspect and override.

The CLI must preserve existing explicit overrides. For example,
`-Profile strict -Tolerance 1.5` must use the strict profile except for the
user-supplied `Tolerance`. Existing long-form commands and compatibility
wrappers must keep working.

## Tech Stack

- Windows PowerShell 5.1+, matching the current `#Requires -Version 5.1`
  headers in the canonical runner and helper library.
- Existing PowerShell runner at `tests/windows/desktop-regression/run.ps1`.
- Existing shared helpers and context construction in
  `tests/windows/desktop-regression/lib/DesktopRegression.ps1`.
- Existing compatibility wrappers in `scripts/desktop-regression/run.ps1` and
  `scripts/window-resize-automation.ps1`.
- Existing Rust binary path default of `target\debug\terminal-manager.exe`
  after `cargo build`.
- Existing artifact root default of `artifacts`, with run output below
  `artifacts/windows/desktop-regression/<run_id>/`.
- Existing suite registration model via `Register-DesktopRegressionSuite`.

## Commands

- List suites, unchanged:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -List`
- List available profiles:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -ListProfiles`
- Run current default behavior through the explicit profile:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Profile default`
- Run strict full desktop regression:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Profile strict`
- Run relaxed full desktop regression against an existing binary:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Profile relaxed -SkipBuild`
- Run a headed smoke subset:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Profile ci-smoke`
- Run snap-focused debugging with an explicit shell override:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Profile snap-debug -SnapShell pwsh`
- Override one profile value without changing the rest of the profile:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Profile strict -Tolerance 1.5`
- Compatibility wrapper, unchanged command shape:
  `powershell.exe -ExecutionPolicy Bypass -File scripts\desktop-regression\run.ps1 -Profile strict -Suite edge-resize-stability`
- Historical resize wrapper, unchanged command shape:
  `powershell.exe -ExecutionPolicy Bypass -File scripts\window-resize-automation.ps1 -OnlySnapTest`

`-List` and `-ListProfiles` must remain non-invasive. They must not initialize
Win32, build the binary, launch suites, move the mouse, send keys, or capture
screenshots.

## Project Structure

- `tests/windows/desktop-regression/run.ps1`: add `-Profile` and
  `-ListProfiles`, resolve the selected profile, merge CLI overrides, and pass
  the resolved values into `New-DesktopRegressionContext`.
- `tests/windows/desktop-regression/lib/DesktopRegression.ps1`: add shared
  profile definition, lookup, validation, merge, and rendering helpers.
- `tests/windows/desktop-regression/SPEC.md`: document profiles as the
  supported way to group runner configuration while preserving flat parameter
  compatibility.
- `tests/windows/desktop-regression/README.md`: document profile commands,
  profile intent, override precedence, and wrapper behavior.
- `scripts/desktop-regression/run.ps1`: add profile-related parameters and
  continue forwarding explicitly bound parameters to the canonical runner.
- `scripts/window-resize-automation.ps1`: keep historical `-OnlySnapTest` and
  `-SkipSnapTest` mapping, but avoid unconditionally forwarding numeric default
  values that would mask a selected profile.
- `specs/desktop-regression-config-profiles.md`: this implementation spec.

No suite scenario files should need to change for the first profile pass.
Suites should continue reading values from the context object.

## Code Style

Use the existing PowerShell style: strict mode, explicit parameters,
`DesktopRegression`-prefixed helper names, ordered hashtables for stable
console output, and `pscustomobject` values for resolved settings. Keep profile
defaults in one place so the runner, docs, and wrappers do not drift.

```powershell
function Get-DesktopRegressionConfigProfiles {
    $base = [ordered]@{
        Suite = $null
        SkipBuild = $false
        ArtifactsDir = "artifacts"
        Tolerance = 2.0
        DragDelta = 220
        SnapLitRatioThreshold = 0.01
        SnapMidLitRatioThreshold = 0.005
        SnapShell = "bash"
        SnapFillLines = 200
        SnapTabbarPx = 88
        SnapStatusbarPx = 32
        SnapSidebarPx = 252
        SnapStripeHeightPx = 12
    }

    return [ordered]@{
        default = $base
        strict = Merge-DesktopRegressionConfig `
            -Base $base `
            -Override @{
                Tolerance = 1.0
                DragDelta = 260
                SnapLitRatioThreshold = 0.015
                SnapMidLitRatioThreshold = 0.003
            }
        relaxed = Merge-DesktopRegressionConfig `
            -Base $base `
            -Override @{
                Tolerance = 4.0
                DragDelta = 180
                SnapLitRatioThreshold = 0.005
                SnapMidLitRatioThreshold = 0.01
            }
        "ci-smoke" = Merge-DesktopRegressionConfig `
            -Base $base `
            -Override @{
                Suite = @("edge-resize-stability")
                Tolerance = 3.0
                DragDelta = 180
            }
        "snap-debug" = Merge-DesktopRegressionConfig `
            -Base $base `
            -Override @{
                Suite = @("post-resize-glitches")
                SnapFillLines = 300
                SnapStripeHeightPx = 8
            }
    }
}

function Resolve-DesktopRegressionConfig {
    param(
        [Parameter(Mandatory = $true)][string]$Profile,
        [Parameter(Mandatory = $true)][hashtable]$BoundParameters
    )

    $profiles = Get-DesktopRegressionConfigProfiles
    if (-not $profiles.Contains($Profile)) {
        $known = ($profiles.Keys -join ", ")
        throw "Unknown desktop regression profile '$Profile'. Known profiles: $known"
    }

    $resolved = Merge-DesktopRegressionConfig -Base $profiles[$Profile]
    foreach ($name in $BoundParameters.Keys) {
        if ($name -in @("Profile", "List", "ListProfiles")) { continue }
        $resolved[$name] = $BoundParameters[$name]
    }

    return [pscustomobject]$resolved
}
```

Conventions:

- Use `default`, `strict`, `relaxed`, `ci-smoke`, and `snap-debug` as stable
  lowercase profile names.
- Treat profile values as runner settings, not suite-specific globals.
- Keep long flat parameters as accepted CLI overrides for compatibility.
- Resolve profiles before path normalization so `ExePath` and `ArtifactsDir`
  follow the same absolute-path logic as today.
- Do not duplicate default values across runner, wrappers, docs, and profile
  helpers unless required for compatibility.
- When wrappers must preserve historical behavior, document each forced value
  in the wrapper rather than hiding it in a profile.

## Testing Strategy

- Verify `-List` still lists suites and exits before profile-sensitive desktop
  execution.
- Verify `-ListProfiles` prints all profile names and resolved values without
  building or launching the app.
- Verify no-profile execution matches the current `default` behavior.
- Verify `-Profile default` produces the same resolved values as no profile.
- Verify `-Profile strict -Tolerance 1.5` keeps strict values except for the
  explicit tolerance override.
- Verify `-Profile relaxed -SkipBuild` skips the build and uses relaxed
  thresholds.
- Verify `-Profile ci-smoke` selects only the smoke suite unless `-Suite`
  overrides it.
- Verify `-Profile snap-debug` selects only `post-resize-glitches` unless
  `-Suite` overrides it.
- Verify unknown profile names fail before build or desktop interaction and
  include known profile names in the error.
- Verify `scripts/desktop-regression/run.ps1` forwards `-Profile` and explicit
  overrides correctly.
- Verify `scripts/window-resize-automation.ps1 -OnlySnapTest` keeps its
  historical suite mapping and does not accidentally mask profile values with
  unbound numeric defaults.

Suggested commands:

- `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -List`
- `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -ListProfiles`
- `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Profile not-real`
- `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Profile strict -Tolerance 1.5 -ListProfiles`
- `powershell.exe -ExecutionPolicy Bypass -File scripts\desktop-regression\run.ps1 -Profile relaxed -ListProfiles`
- `powershell.exe -ExecutionPolicy Bypass -File scripts\window-resize-automation.ps1 -OnlySnapTest`

Full desktop suite execution remains manual because these tests control the
local desktop.

## Boundaries

- Always: keep the current no-profile command behavior equivalent to the new
  `default` profile.
- Always: let explicitly supplied CLI parameters override profile values.
- Always: keep existing flat parameters accepted by the canonical runner and
  wrappers.
- Always: keep `-List` and `-ListProfiles` non-invasive.
- Always: include profile name and resolved settings in run output or artifacts
  so failures can be reproduced.
- Always: validate unknown profiles before building, launching, or touching the
  desktop.
- Ask first: removing any existing flat runner parameter.
- Ask first: adding profiles beyond `default`, `strict`, `relaxed`,
  `ci-smoke`, and `snap-debug`.
- Ask first: adding this framework or `ci-smoke` to actual CI/CD workflows.
- Ask first: changing suite behavior or screenshot sampling logic as part of
  profile support.
- Never: make wrappers silently override explicit profile choices with their
  own unbound default values.
- Never: move profile definitions into individual suite files.
- Never: treat `ci-smoke` as permission to run headed desktop tests on an
  unattended or non-interactive machine.

## Plan

1. Define the profile data model.
   - Add a single ordered profile map with `default`, `strict`, `relaxed`,
     `ci-smoke`, and `snap-debug`.
   - Keep current runner defaults in `default`.
   - Allow profiles to set `Suite`, `SkipBuild`, `ArtifactsDir`, geometry
     tolerance, drag distance, snap shell, snap sampling geometry, and snap
     thresholds.

2. Add profile resolution helpers.
   - Add a merge helper that clones an ordered base map before applying
     overrides.
   - Add a resolver that loads the selected profile and applies only explicit
     entries from `$PSBoundParameters`.
   - Add a renderer for `-ListProfiles` that prints stable, human-readable
     resolved values.

3. Wire profiles into the canonical runner.
   - Add `[ValidateSet("default", "strict", "relaxed", "ci-smoke",
     "snap-debug")] [string]$Profile = "default"`.
   - Add `[switch]$ListProfiles`.
   - Resolve profile settings after suite registration and before build/path
     checks.
   - Use the resolved settings for suite selection, build skip, artifact root,
     and `New-DesktopRegressionContext`.
   - Include the selected profile in console output and `results.json` if the
     result schema allows it without breaking existing consumers.

4. Update wrappers without breaking old commands.
   - Add `-Profile` and `-ListProfiles` to
     `scripts/desktop-regression/run.ps1` and keep forwarding
     `$PSBoundParameters`.
   - In `scripts/window-resize-automation.ps1`, preserve historical suite
     mapping and `-SkipBuild` behavior, but forward profile-controlled scalar
     values only when the user explicitly supplied them.

5. Document the contract.
   - Update `SPEC.md` with profile precedence and boundaries.
   - Update `README.md` with commands and a concise profile table.
   - Keep documentation clear that this remains a manual desktop framework.

6. Verify behavior.
   - Run non-invasive list/profile commands first.
   - Exercise override precedence through the canonical runner and wrappers.
   - Run selected full desktop profiles manually only on an interactive Windows
     desktop.

## Tasks

- [ ] Task: Add profile map and merge helpers.
  - Acceptance: `lib/DesktopRegression.ps1` exposes stable helpers for listing
    profiles, merging profile settings, and resolving explicit overrides.
  - Verify: Dot-source the library and inspect the resolved `default`,
    `strict`, `relaxed`, `ci-smoke`, and `snap-debug` settings.
  - Files: `tests/windows/desktop-regression/lib/DesktopRegression.ps1`.

- [ ] Task: Add `-Profile` and `-ListProfiles` to the canonical runner.
  - Acceptance: `run.ps1` accepts the five profile names, rejects unknown
    names, prints profiles non-invasively, and uses resolved settings for
    context creation.
  - Verify: Run `-List`, `-ListProfiles`, `-Profile default`, and
    `-Profile not-real`.
  - Files: `tests/windows/desktop-regression/run.ps1`.

- [ ] Task: Preserve explicit CLI override precedence.
  - Acceptance: Any explicit flat parameter in `$PSBoundParameters` overrides
    the selected profile while omitted flat parameters use profile values.
  - Verify: Compare resolved output for `-Profile strict`,
    `-Profile strict -Tolerance 1.5`, and
    `-Profile snap-debug -Suite edge-resize-stability`.
  - Files: `tests/windows/desktop-regression/run.ps1`,
    `tests/windows/desktop-regression/lib/DesktopRegression.ps1`.

- [ ] Task: Update compatibility wrappers.
  - Acceptance: `scripts/desktop-regression/run.ps1` forwards profile options,
    and `scripts/window-resize-automation.ps1` preserves old commands without
    masking profile settings with unbound defaults.
  - Verify: Run wrapper `-ListProfiles`, wrapper `-Profile relaxed -List`, and
    historical `scripts\window-resize-automation.ps1 -OnlySnapTest`.
  - Files: `scripts/desktop-regression/run.ps1`,
    `scripts/window-resize-automation.ps1`.

- [ ] Task: Update framework docs.
  - Acceptance: `SPEC.md` and `README.md` explain profile names, intended use,
    override precedence, wrapper behavior, and the manual desktop boundary.
  - Verify: Follow each documented non-invasive command exactly.
  - Files: `tests/windows/desktop-regression/SPEC.md`,
    `tests/windows/desktop-regression/README.md`.

- [ ] Task: Run manual profile checks.
  - Acceptance: `default`, `strict`, `relaxed`, `ci-smoke`, and `snap-debug`
    resolve and run as intended on a real interactive Windows desktop.
  - Verify: Run each profile or a representative selected suite, then inspect
    console output and `results.json` for the selected profile and settings.
  - Files: no additional source files unless verification exposes a bug.

## Success Criteria

- The runner supports `-Profile default`, `-Profile strict`,
  `-Profile relaxed`, `-Profile ci-smoke`, and `-Profile snap-debug`.
- Running without `-Profile` preserves the current default behavior.
- Existing flat parameters remain accepted and override selected profile
  values when explicitly supplied.
- `-ListProfiles` is non-invasive and shows enough information to reproduce a
  profile run.
- Unknown profile names fail early with a clear list of known profiles.
- Compatibility wrappers keep old command shapes working and can pass profile
  choices through to the canonical runner.
- Profile support does not require edits to existing suite files.
- Documentation points users toward profiles while still documenting explicit
  overrides for unusual local machines.

## Open Questions

- Should `-ListProfiles` show raw profile values only, or should it also show
  fully normalized paths for `ExePath` and `ArtifactsDir`?
- Should `ci-smoke` select only `edge-resize-stability`, only
  `post-resize-glitches`, or both once preflight and lifecycle isolation specs
  are implemented?
- Should profile metadata be written into `results.json`, a separate
  `config.json`, or both?
- Should wrapper-specific historical behavior such as forced `-SkipBuild` in
  `scripts/window-resize-automation.ps1` be represented as a hidden profile or
  kept as explicit wrapper logic?
- Should profiles eventually support external JSON files, or should they remain
  hard-coded PowerShell maps until more use cases exist?
