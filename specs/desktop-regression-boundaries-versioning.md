# Spec: Desktop Regression Boundaries And Versioning

## Objective

Define the public boundaries and versioning policy for
`tests/windows/desktop-regression` so engineers and agents can evolve the manual
Windows desktop regression framework without breaking existing local workflows,
artifact consumers, or suite authors unexpectedly.

The current framework has a canonical runner at
`tests/windows/desktop-regression/run.ps1`, a shared helper library at
`tests/windows/desktop-regression/lib/DesktopRegression.ps1`, and historical
entry points under `scripts/`. It drives a real Windows desktop through Win32,
global mouse and keyboard input, screenshots, DPI-aware geometry, and pixel
checks. These capabilities make the framework useful, but they also require
clear support limits.

Success means the framework has a documented contract for supported
PowerShell/Windows versions, launch modes, artifact schema versions,
compatibility wrappers, future operating-system boundaries, manual-vs-CI usage,
and public helper API changes.

## Tech Stack

- Windows PowerShell 5.1, matching the existing `#Requires -Version 5.1`
  declarations in the canonical runner, helper library, and compatibility
  wrappers.
- Windows desktop APIs through `user32.dll` interop in
  `DesktopRegressionWin32`.
- .NET `System.Drawing` and `System.Windows.Forms` for screenshots, screen
  geometry, DPI-aware behavior, and synthesized keyboard input.
- Rust build output from `cargo build`, with the default binary at
  `target\debug\terminal-manager.exe`.
- Canonical manual runner:
  `tests\windows\desktop-regression\run.ps1`.
- Compatibility wrappers:
  `scripts\desktop-regression\run.ps1` and
  `scripts\window-resize-automation.ps1`.

Supported runtime modes:

- PowerShell host: `powershell.exe` on Windows PowerShell 5.1 is supported.
- PowerShell Core: `pwsh.exe` is not a supported runner host until the Win32,
  `System.Drawing`, `System.Windows.Forms`, and SendKeys behavior is explicitly
  validated and documented.
- Windows desktop: interactive Windows sessions with a visible desktop are
  supported. Headless sessions, service sessions, non-interactive remoting, WSL,
  and Linux/macOS shells are out of scope.
- App launch: default build-and-launch, `-SkipBuild` with the default binary,
  explicit relative or absolute `-ExePath`, and wrapper-forwarded launch modes
  are supported.
- Shell fill commands inside the app: `bash`, `pwsh`, and `cmd` are the
  supported `-SnapShell` values because `Send-DesktopRegressionFillCommand` and
  `Send-DesktopRegressionClearCommand` define those branches today.

## Commands

- List canonical suites without build, artifact creation, or desktop control:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -List`
- Run every canonical suite with the default build step:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1`
- Run one canonical suite:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Suite post-resize-glitches`
- Run against the default existing binary without building:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -SkipBuild`
- Run against an explicit binary path:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -SkipBuild -ExePath target\debug\terminal-manager.exe`
- Write artifacts under an explicit root:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -ArtifactsDir artifacts`
- Validate the modern compatibility wrapper:
  `powershell.exe -ExecutionPolicy Bypass -File scripts\desktop-regression\run.ps1 -List`
- Validate the historical resize wrapper:
  `powershell.exe -ExecutionPolicy Bypass -File scripts\window-resize-automation.ps1 -OnlySnapTest`
- Inspect the current result artifact shape after a run:
  `powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "$d = Get-ChildItem artifacts\windows\desktop-regression -Directory | Sort-Object Name -Descending | Select-Object -First 1; Get-Content -Raw (Join-Path $d.FullName 'results.json') | ConvertFrom-Json | Format-List"`

## Project Structure

- `tests/windows/desktop-regression/run.ps1`: canonical public runner, command
  line parameters, suite discovery, optional `cargo build`, app binary path
  resolution, artifact root resolution, suite execution, console status, exit
  code, and `results.json` writing.
- `tests/windows/desktop-regression/lib/DesktopRegression.ps1`: public helper
  library for suite registration, context construction, Win32 initialization,
  process/window lifecycle, screenshots, input, pixel sampling, assertions, and
  artifact paths.
- `tests/windows/desktop-regression/suites/*.ps1`: framework consumers that
  register suites through `Register-DesktopRegressionSuite`.
- `tests/windows/desktop-regression/SPEC.md`: existing framework contract and
  manual desktop-test boundaries.
- `tests/windows/desktop-regression/README.md`: user-facing commands,
  compatibility wrapper examples, layout, and suite authoring flow.
- `scripts/desktop-regression/run.ps1`: compatibility wrapper that forwards
  modern runner parameters to the canonical path.
- `scripts/window-resize-automation.ps1`: historical compatibility wrapper that
  maps `-OnlySnapTest` and `-SkipSnapTest` to canonical suite names.
- `artifacts/windows/desktop-regression/<run_id>/`: generated run artifacts,
  including screenshots and `results.json`.
- `specs/desktop-regression-boundaries-versioning.md`: this spec, the source
  of truth for framework compatibility and versioning policy.

## Code Style

Keep public contract checks explicit and close to the runner or helper API that
owns them. Use PowerShell 5.1-compatible syntax, stable function names, exact
error messages, and ordered objects for versioned artifacts. Prefer additive
fields and wrapper forwarding over caller-visible rewrites.

```powershell
function New-DesktopRegressionArtifactEnvelope {
    param(
        [Parameter(Mandatory = $true)][string]$SchemaVersion,
        [Parameter(Mandatory = $true)][string]$RunnerVersion,
        [Parameter(Mandatory = $true)][object[]]$Suites
    )

    if ($SchemaVersion -notmatch '^desktop-regression\.results/v[0-9]+$') {
        throw "Invalid desktop regression artifact schema version: $SchemaVersion"
    }

    return [pscustomobject][ordered]@{
        schema_version = $SchemaVersion
        runner_version = $RunnerVersion
        compatibility = [pscustomobject][ordered]@{
            runner_path = "tests/windows/desktop-regression/run.ps1"
            powershell = "5.1"
            os_family = "windows"
            mode = "manual-desktop"
        }
        suites = @($Suites)
    }
}
```

Conventions:

- Public PowerShell helper names use `Verb-DesktopRegressionNoun`.
- Public JSON keys use snake_case.
- Public version identifiers use namespaced strings such as
  `desktop-regression.results/v1` and `desktop-regression.helpers/v1`.
- Breaking changes require a new version identifier and documentation before
  implementation.
- Deprecated wrapper paths must warn only after a documented migration window is
  approved.

## Testing Strategy

- Use `run.ps1 -List` as the primary non-invasive smoke test for canonical
  runner availability. It must not initialize Win32, build, move input, capture
  the screen, or create run artifacts.
- Use `scripts\desktop-regression\run.ps1 -List` to verify the modern wrapper
  forwards every supported public parameter without changing output semantics.
- Use `scripts\window-resize-automation.ps1 -OnlySnapTest` only as a manual
  compatibility check because it runs a real suite and controls the desktop.
- Add helper-level validation for version strings, supported launch-mode
  normalization, artifact envelope construction, and compatibility policy
  warnings without launching `terminal-manager.exe`.
- When a versioned artifact schema is implemented, validate each generated
  `results.json` with `ConvertFrom-Json` and assert the documented
  `schema_version`, runner compatibility metadata, and suite result fields.
- Full desktop suite execution remains manual because the suites control native
  windows, move the cursor, send keys such as `Win+Left`, and capture the
  visible desktop.

## Boundaries

Always:

- Treat `tests\windows\desktop-regression\run.ps1` as the canonical public
  runner path.
- Support Windows PowerShell 5.1 through `powershell.exe` for the runner,
  wrappers, and helper library.
- Support interactive Windows desktop execution only.
- Preserve `-List`, `-Suite`, `-SkipBuild`, `-ExePath`, `-ArtifactsDir`,
  `-Tolerance`, `-DragDelta`, and the existing snap-related parameters unless a
  versioned deprecation is documented first.
- Support app launch through the current default binary path, explicit
  `-ExePath`, and `-SkipBuild` workflows.
- Keep wrapper scripts under `scripts/` forwarding to the canonical runner while
  they are documented in `README.md`.
- Keep result artifacts under
  `artifacts/windows/desktop-regression/<run_id>/` by default.
- Version any machine-readable artifact schema with a top-level
  `schema_version` before consumers are expected to parse it as a stable
  contract.
- Document public helper API additions, deprecations, and breaking changes in
  `SPEC.md` or `README.md` before suite authors are expected to adopt them.

Ask first:

- Adding CI execution for full desktop suites or changing workflow files to run
  them automatically.
- Expanding supported runner hosts to `pwsh.exe`.
- Expanding supported operating systems beyond Windows.
- Removing either compatibility wrapper or adding warnings that could break
  scripts expecting exact output.
- Changing historical wrapper semantics for `-OnlySnapTest`, `-SkipSnapTest`,
  or `-SkipBuild`.
- Introducing external PowerShell modules, native dependencies, or a new test
  framework for boundary/version checks.
- Bumping an artifact or helper API major version.

Never:

- Run full desktop suites as normal CI jobs without an explicit approved
  desktop-runner design.
- Treat browser e2e tests, in-process Rust tests, or unit tests as consumers of
  this framework.
- Share the Windows helper library with future macOS or Linux desktop
  frameworks. Future OS frameworks must live in sibling OS directories.
- Break existing documented commands silently.
- Write unversioned machine-readable artifact formats once a stable schema is
  declared.
- Remove a public helper function used by suites without either a compatibility
  shim or a documented major-version migration.

Supported app launch modes:

- `build-default`: `run.ps1` runs `cargo build`, then launches
  `target\debug\terminal-manager.exe`.
- `skip-build-default`: `run.ps1 -SkipBuild` launches the existing default
  binary.
- `explicit-exe`: `run.ps1 -ExePath <path>` launches a relative or absolute
  binary path after resolving it from the repository root when needed.
- `wrapper-forwarded`: `scripts\desktop-regression\run.ps1` forwards supported
  parameters to the canonical runner.
- `historical-resize-wrapper`: `scripts\window-resize-automation.ps1` maps
  legacy resize flags to `edge-resize-stability` and
  `post-resize-glitches`.

Artifact schema versioning:

- Current state: `results.json` is a flat JSON array containing suite `name`,
  `title`, `status`, `duration_seconds`, and optional `error`.
- First stable version: when consumers need a documented machine-readable
  contract, write a root object with
  `schema_version = "desktop-regression.results/v1"`.
- Additive fields in a schema version are allowed only when existing required
  fields keep their names, types, and meanings.
- Removing fields, renaming fields, changing field types, changing status
  values, or changing artifact path semantics requires the next major schema
  string, such as `desktop-regression.results/v2`.
- Result readers should accept known older schema versions during a documented
  compatibility window.
- Human console output is not the artifact schema and should remain optimized
  for local readability.

Compatibility policy for old script paths:

- `scripts\desktop-regression\run.ps1` is a supported compatibility wrapper as
  long as it is documented in `README.md`.
- `scripts\window-resize-automation.ps1` is a legacy compatibility wrapper for
  existing local commands and agent notes.
- Compatibility wrappers should forward to the canonical runner instead of
  duplicating behavior.
- New documentation should prefer
  `tests\windows\desktop-regression\run.ps1`.
- Wrapper removal requires a documented deprecation plan, a replacement command,
  and approval because external notes may still refer to those paths.

Future OS boundaries:

- The Windows framework owns only `tests/windows/desktop-regression`.
- Future macOS or Linux desktop frameworks should use sibling directories such
  as `tests/macos/desktop-regression` or `tests/linux/desktop-regression`.
- Cross-OS behavior should be documented at a higher level, not implemented by
  branching inside the Windows Win32 helper library.

Manual-vs-CI boundaries:

- `-List` and helper-level validation are eligible for automated checks because
  they do not control the desktop.
- Full suite execution is manual by default.
- CI execution requires a separate approved design for an interactive Windows
  desktop agent, display isolation, input isolation, artifacts, timing
  tolerances, and failure triage.

Public helper API versioning:

- Public helpers are functions intended for suite files, such as
  `Register-DesktopRegressionSuite`, `New-DesktopRegressionContext`,
  `Start-DesktopRegressionApp`, `Stop-DesktopRegressionApp`,
  `New-DesktopRegressionArtifactPath`, screenshot helpers, input helpers, and
  assertion helpers.
- Additive helper functions are minor-version changes and must be documented
  with examples before use in templates.
- Additive optional parameters are minor-version changes when defaults preserve
  current behavior.
- Renaming helpers, changing required parameters, changing return object fields,
  changing assertion semantics, or changing context property names is a
  major-version change.
- Deprecated helpers should remain as shims for at least one documented
  migration window unless removal is explicitly approved.

## Success Criteria

- The canonical runner path, supported wrappers, supported PowerShell host,
  supported Windows execution mode, and supported app launch modes are
  documented in one place.
- The framework has a clear artifact schema policy that distinguishes the
  current flat `results.json` from future stable
  `desktop-regression.results/vN` schemas.
- Old script paths have an explicit compatibility and deprecation policy.
- Future macOS and Linux desktop testing work has a documented boundary that
  keeps OS-specific helpers out of the Windows Win32 library.
- Manual checks, non-invasive automated checks, and future CI execution are
  clearly separated.
- Public helper API additions, deprecations, and breaking changes have a
  documented versioning policy.
- Existing documented commands from `SPEC.md` and `README.md` remain compatible.

## Open Questions

- Should the framework expose an explicit `-RunnerVersion` or derive the helper
  API version from constants in `DesktopRegression.ps1`?
- Should compatibility wrappers print deprecation warnings eventually, or stay
  silent to protect exact-output scripts?
- What is the minimum supported Windows release for desktop input, snap, DPI,
  and screenshot behavior: Windows 10, Windows 11, or only the development
  team's active Windows version?
- Should schema readers be implemented inside this repository, or is the
  versioned artifact contract only for external tooling?
- How long should a helper API shim remain after a major-version replacement?

## Plan

1. Define constants or helper functions for runner version, helper API version,
   supported PowerShell host, supported OS family, and artifact schema version.
2. Add non-invasive boundary checks that run during `-List` or helper-level
   tests without initializing Win32 or controlling the desktop.
3. Introduce a versioned artifact envelope only when the result schema work
   moves beyond the current flat array format.
4. Document compatibility wrapper status, deprecation policy, and preferred
   canonical commands in `README.md` and `SPEC.md`.
5. Add migration notes for any public helper API change before changing suites
   or templates to depend on it.

Risks and mitigations:

- Risk: documenting versions without enforcing them can create drift. Mitigate
  with helper-level checks that read the exported constants and validate
  generated artifacts.
- Risk: compatibility wrappers hide canonical runner changes. Mitigate by
  keeping wrappers thin and testing `-List` forwarding.
- Risk: CI pressure leads to unreliable desktop jobs. Mitigate by allowing only
  non-invasive checks until an interactive Windows runner design is approved.

Verification checkpoints:

- `run.ps1 -List` still works before any Win32 initialization or build.
- `scripts\desktop-regression\run.ps1 -List` still forwards successfully.
- Any future versioned `results.json` declares its schema version at the root.
- Documentation changes name both supported wrapper paths and the canonical path.

## Tasks

- [ ] Task: Add framework version constants.
  - Acceptance: the helper library exposes stable values for runner contract,
    helper API version, supported PowerShell host, and supported OS family
    without changing suite behavior.
  - Verify: dot-source `lib\DesktopRegression.ps1` in PowerShell 5.1 and print
    the values without initializing Win32.
  - Files: `tests/windows/desktop-regression/lib/DesktopRegression.ps1`.

- [ ] Task: Add non-invasive boundary validation.
  - Acceptance: unsupported host or OS checks produce clear messages before
    build, artifact creation, or desktop control.
  - Verify:
    `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -List`
    remains non-invasive and succeeds on supported Windows PowerShell 5.1.
  - Files: `tests/windows/desktop-regression/run.ps1`,
    `tests/windows/desktop-regression/lib/DesktopRegression.ps1`.

- [ ] Task: Define the stable artifact schema envelope.
  - Acceptance: when result schema versioning is implemented, `results.json`
    declares `schema_version`, runner compatibility metadata, and suite results
    in a root object while documented older formats remain readable for the
    migration window.
  - Verify: parse the latest `results.json` with `ConvertFrom-Json` and assert
    the expected schema/version fields.
  - Files: `tests/windows/desktop-regression/run.ps1`,
    `tests/windows/desktop-regression/lib/DesktopRegression.ps1`.

- [ ] Task: Document wrapper compatibility and deprecation policy.
  - Acceptance: README and framework spec state the canonical path, both
    compatibility paths, preferred new commands, and the approval requirement
    before wrapper removal or warning changes.
  - Verify: manually compare documented commands with the wrapper parameter
    lists and run the modern wrapper `-List` command.
  - Files: `tests/windows/desktop-regression/README.md`,
    `tests/windows/desktop-regression/SPEC.md`,
    `scripts/desktop-regression/run.ps1`,
    `scripts/window-resize-automation.ps1`.

- [ ] Task: Document public helper API migration rules.
  - Acceptance: suite authors can tell whether a helper change is additive,
    deprecated, or breaking, and where to find migration notes.
  - Verify: review current suite files against the public helper list and
    confirm no current helper usage is undocumented.
  - Files: `tests/windows/desktop-regression/README.md`,
    `tests/windows/desktop-regression/SPEC.md`,
    `tests/windows/desktop-regression/templates/suite.ps1`.
