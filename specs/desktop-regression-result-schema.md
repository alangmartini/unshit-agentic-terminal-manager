# Spec: Desktop Regression Result Schema

## Objective

Define a versioned, machine-readable result schema for
`tests/windows/desktop-regression` runs.

The current runner discovers registered PowerShell suites, optionally builds
`target\debug\terminal-manager.exe`, executes selected suites, prints human
status lines, and writes a flat `results.json` array with suite name, title,
status, duration, and optional error text. The new schema should keep that
manual workflow intact while making each run artifact self-describing enough to
compare runs, diagnose environment-specific failures, and attach evidence to a
regression report.

The first supported schema version is `desktop-regression.results/v1`.
`results.json` should become a root object with versioned run metadata,
environment metadata, binary identity, suite metadata, suite artifacts, failure
classification, structured events, and timing.

## Tech Stack

- Windows PowerShell 5.1, matching the `#Requires -Version 5.1` runner and
  library.
- Existing Win32 interop in `lib/DesktopRegression.ps1` via `Add-Type`.
- Existing .NET `System.Drawing` and `System.Windows.Forms` usage for screen
  capture, screen bounds, and DPI-aware desktop interaction.
- Built-in PowerShell commands only: `ConvertTo-Json`, `ConvertFrom-Json`,
  `Get-FileHash`, `Get-CimInstance`, `Get-Date`, and `Measure-Command` or
  `System.Diagnostics.Stopwatch`.
- Existing Rust build command: `cargo build`.
- No new package dependencies for v1 schema writing or validation.

## Commands

- List suites without creating run artifacts:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -List`
- Run all suites and write a v1 `results.json` artifact:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1`
- Run one suite against an existing binary:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -SkipBuild -ExePath target\debug\terminal-manager.exe -Suite post-resize-glitches`
- Run one suite with an explicit artifact root:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Suite edge-resize-stability -ArtifactsDir artifacts`
- Validate the latest result artifact shape:
  `powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "$d = Get-ChildItem artifacts\windows\desktop-regression -Directory | Sort-Object Name -Descending | Select-Object -First 1; $r = Get-Content -Raw (Join-Path $d.FullName 'results.json') | ConvertFrom-Json; if ($r.schema_version -ne 'desktop-regression.results/v1') { throw 'bad schema_version' }; if (-not $r.run.id) { throw 'missing run.id' }"`

## Project Structure

- `tests/windows/desktop-regression/run.ps1`: owns command-line parameters,
  suite discovery, optional build, selected suite execution, top-level run
  timing, result aggregation, and final `results.json` writing.
- `tests/windows/desktop-regression/lib/DesktopRegression.ps1`: owns shared
  helpers for Win32 setup, screen size, DPI-aware behavior, process/window
  lifecycle, screenshots, artifact paths, assertions, and future metadata/event
  helper functions.
- `tests/windows/desktop-regression/suites/*.ps1`: continue to register suites
  with `Register-DesktopRegressionSuite -Name -Title -Covers -Tags
  -ScriptBlock`; suite tags in result artifacts must come from this registration
  data.
- `tests/windows/desktop-regression/SPEC.md`: framework contract and manual
  desktop-test boundaries; update when implementation lands.
- `tests/windows/desktop-regression/README.md`: user-facing command and artifact
  documentation; update when implementation lands.
- `artifacts/windows/desktop-regression/<run_id>/results.json`: canonical v1
  result file for a run.
- `artifacts/windows/desktop-regression/<run_id>/*.png`: screenshots captured by
  suites and referenced from `results.json` by path relative to the run artifact
  directory.

## Schema Contract

`results.json` should use stable snake_case JSON keys and preserve field order
with `[ordered]` hashtables before calling `ConvertTo-Json -Depth 10`.

Top-level shape:

```json
{
  "schema_version": "desktop-regression.results/v1",
  "run": {
    "id": "20260508-211530",
    "status": "passed",
    "started_at_utc": "2026-05-09T00:15:30.0000000Z",
    "finished_at_utc": "2026-05-09T00:16:02.0000000Z",
    "duration_ms": 32000,
    "command": "powershell.exe -ExecutionPolicy Bypass -File tests\\windows\\desktop-regression\\run.ps1 -Suite post-resize-glitches",
    "repo_root": "C:\\Users\\Alan Beelink\\dev\\unshit-test-framework",
    "artifacts_dir": "artifacts\\windows\\desktop-regression\\20260508-211530",
    "selected_suites": ["post-resize-glitches"],
    "parameters": {
      "skip_build": false,
      "tolerance": 2.0,
      "drag_delta": 220,
      "snap_shell": "bash"
    }
  },
  "source": {
    "commit_sha": "full git commit sha when available",
    "working_tree_dirty": true
  },
  "binary": {
    "path": "C:\\Users\\Alan Beelink\\dev\\unshit-test-framework\\target\\debug\\terminal-manager.exe",
    "sha256": "hex sha256 when the file exists",
    "built_by_runner": true,
    "build_duration_ms": 12000
  },
  "environment": {
    "os": {
      "caption": "Microsoft Windows 11 Pro",
      "version": "10.0.22631",
      "build_number": "22631",
      "architecture": "64-bit"
    },
    "screen": {
      "width_px": 2560,
      "height_px": 1440,
      "primary_dpi_x": 96.0,
      "primary_dpi_y": 96.0,
      "scale_percent": 100
    }
  },
  "summary": {
    "total": 1,
    "passed": 1,
    "failed": 0,
    "skipped": 0
  },
  "suites": []
}
```

Suite result shape:

```json
{
  "name": "post-resize-glitches",
  "title": "Post-resize visual stability",
  "covers": "Aero snap growth, bottom statusbar reflow, and stale terminal rows.",
  "tags": ["snap", "resize", "visual"],
  "status": "failed",
  "timing": {
    "started_at_utc": "2026-05-09T00:15:31.0000000Z",
    "finished_at_utc": "2026-05-09T00:15:49.0000000Z",
    "duration_ms": 18000
  },
  "failure": {
    "type": "assertion",
    "message": "bottom statusbar differs by 9 > 2",
    "exception_type": "RuntimeException",
    "stack_excerpt": "short runner-safe stack excerpt"
  },
  "artifacts": {
    "screenshots": [
      {
        "label": "after-snap",
        "path": "post-resize-glitches-after-snap.png",
        "sha256": "hex sha256 when the file exists",
        "width_px": 2560,
        "height_px": 1440
      }
    ],
    "stdout_excerpt": [],
    "stderr_excerpt": [],
    "events": [
      {
        "timestamp_utc": "2026-05-09T00:15:36.0000000Z",
        "level": "info",
        "message": "captured screenshot",
        "data": {
          "path": "post-resize-glitches-after-snap.png"
        }
      }
    ]
  }
}
```

Failure `type` must be one of:

- `assertion`: a framework assertion failed, such as
  `Assert-DesktopRegressionClose` or `Assert-DesktopRegressionTrue`.
- `app_launch`: the binary was missing, failed to start, or no expected native
  window appeared.
- `timeout`: a wait loop exceeded its deadline.
- `process_exit`: the application exited before the suite finished.
- `harness_error`: runner, metadata, filesystem, screenshot, or Win32 helper
  failure unrelated to the product behavior under test.
- `unknown`: fallback when the runner cannot classify the exception.

For passed suites, `failure` must be `null`. For failed suites, `failure.type`
and `failure.message` are required.

## Code Style

Use small PowerShell functions that return `[pscustomobject]` values backed by
`[ordered]` hashtables. Do not build JSON with string concatenation. Keep public
JSON keys in snake_case to match the existing `duration_seconds` style in the
runner.

```powershell
function New-DesktopRegressionSuiteResult {
    param(
        [Parameter(Mandatory = $true)]$Suite,
        [Parameter(Mandatory = $true)][DateTime]$StartedAtUtc,
        [Parameter(Mandatory = $true)][DateTime]$FinishedAtUtc,
        [Parameter(Mandatory = $true)][TimeSpan]$Duration,
        [AllowNull()]$Failure,
        [object[]]$Screenshots = @(),
        [object[]]$Events = @(),
        [string[]]$StdoutExcerpt = @(),
        [string[]]$StderrExcerpt = @()
    )

    $status = if ($Failure) { "failed" } else { "passed" }

    return [pscustomobject][ordered]@{
        name = $Suite.Name
        title = $Suite.Title
        covers = $Suite.Covers
        tags = @($Suite.Tags)
        status = $status
        timing = [pscustomobject][ordered]@{
            started_at_utc = $StartedAtUtc.ToUniversalTime().ToString("o")
            finished_at_utc = $FinishedAtUtc.ToUniversalTime().ToString("o")
            duration_ms = [int][Math]::Round($Duration.TotalMilliseconds)
        }
        failure = $Failure
        artifacts = [pscustomobject][ordered]@{
            screenshots = @($Screenshots)
            stdout_excerpt = @($StdoutExcerpt)
            stderr_excerpt = @($StderrExcerpt)
            events = @($Events)
        }
    }
}
```

## Testing Strategy

- Preserve `-List` as a non-invasive smoke test. It must not require metadata
  collection, build the binary, or create `results.json`.
- Add a schema smoke check that runs one suite with `-SkipBuild`, parses the
  latest `results.json` with `ConvertFrom-Json`, and asserts required v1 fields.
- Add a failure-path validation by running a deliberately failing local fixture
  suite or by unit-testing result construction helpers, then assert
  `failure.type`, `failure.message`, and suite timing are present.
- Verify screenshot references by checking that every
  `suites[].artifacts.screenshots[].path` is relative to the run artifact
  directory and points to an existing file with a matching SHA-256 hash.
- Validate `source.commit_sha` using `git rev-parse HEAD` when Git is available;
  if unavailable, the field should be `null` and an explanatory structured event
  should be present.
- Full desktop suite execution remains manual because it controls the real
  desktop, sends global input such as `Win+Left`, moves the mouse, and captures
  screenshots.

## Boundaries

- Always: keep `run.ps1` command-line compatibility for `-List`, `-Suite`,
  `-SkipBuild`, `-ExePath`, and `-ArtifactsDir`.
- Always: write artifacts under the resolved run artifact directory created by
  `New-DesktopRegressionContext`.
- Always: include `schema_version`, run timing, OS, screen size, DPI, commit SHA,
  binary path/hash, suite tags, suite timing, failure classification, and
  screenshot/event artifact references when available.
- Always: use structured JSON objects and arrays, not formatted console text, as
  the source of truth for result data.
- Ask first: adding external PowerShell modules, changing CI/CD, or running full
  desktop suites on behalf of a user.
- Ask first: changing suite registration shape beyond additive fields.
- Never: capture unrelated desktop screenshots outside explicit suite helper
  calls.
- Never: write local environment variables, API keys, tokens, or arbitrary
  process command lines into result artifacts.
- Never: remove the existing human-readable console output unless a replacement
  is explicitly approved.

## Success Criteria

- `results.json` is a root JSON object with
  `schema_version = "desktop-regression.results/v1"` rather than a flat suite
  array.
- Each run records stable metadata for command parameters, selected suites,
  artifact directory, start/end timestamps, total duration, summary counts, OS,
  screen size, DPI, Git commit SHA, working tree dirty state, binary path,
  binary SHA-256, and build timing when the runner builds the binary.
- Every suite result records name, title, covers, tags, status, timing, failure
  object or `null`, screenshot artifact references, stdout/stderr excerpts when
  captured, and structured events.
- Failed suites include a non-empty `failure.type` from the documented enum and
  a non-empty `failure.message`.
- Existing manual commands from `README.md` and `SPEC.md` still work.
- `-List` output is unchanged except for harmless formatting fixes, and it does
  not create run artifacts.

## Open Questions

- Should v1 capture child process stdout/stderr by launching
  `terminal-manager.exe` with redirected streams, or should suites rely on
  structured runner events until process logging is explicitly needed?
- Should DPI be primary-monitor only for v1, or should the schema capture all
  attached monitors and the window monitor for each screenshot?
- Should `repo_root` and `binary.path` be absolute paths for reproducibility, or
  relative paths for easier artifact sharing?
- Should a dirty working tree record only a boolean, or also a short list of
  changed file paths?
- Should screenshot width/height be read back from the PNG file for accuracy, or
  is the captured screen size sufficient?

## Plan

1. Define helper functions for run metadata, source metadata, binary metadata,
   display/DPI metadata, failure classification, screenshot artifact metadata,
   and structured events.
2. Extend the desktop regression context so suites and helpers can append
   screenshots and events without each suite manually shaping JSON.
3. Update `run.ps1` to measure build timing, run timing, and suite timing with
   UTC timestamps and millisecond durations.
4. Replace the flat `results.json` array writer with the v1 root object while
   preserving console output and exit codes.
5. Add smoke validation for the schema and update desktop regression
   documentation after implementation.

## Tasks

- [ ] Task: Add metadata helper functions.
  Acceptance: helpers return ordered PowerShell objects for OS, screen/DPI,
  Git source state, binary identity, and failure classification.
  Verify: run a PowerShell command that dot-sources
  `tests\windows\desktop-regression\lib\DesktopRegression.ps1`, calls each
  helper, serializes with `ConvertTo-Json -Depth 10`, and parses it back with
  `ConvertFrom-Json`.
  Files: `tests/windows/desktop-regression/lib/DesktopRegression.ps1`.

- [ ] Task: Add suite artifact and event collection to the context.
  Acceptance: screenshot helper calls can register label, relative path,
  dimensions, and SHA-256; suites can append structured events without knowing
  the final result schema.
  Verify: run one suite and confirm captured screenshots appear in
  `suites[].artifacts.screenshots`.
  Files: `tests/windows/desktop-regression/lib/DesktopRegression.ps1`,
  `tests/windows/desktop-regression/run.ps1`.

- [ ] Task: Replace the runner result writer with v1 `results.json`.
  Acceptance: a run writes one root object with `schema_version`, `run`,
  `source`, `binary`, `environment`, `summary`, and `suites`.
  Verify: run the latest-result validation command from this spec and confirm
  failures still exit with code 1.
  Files: `tests/windows/desktop-regression/run.ps1`.

- [ ] Task: Validate failure typing and timing.
  Acceptance: assertion failures are classified as `assertion`; launch/window
  failures are classified as `app_launch` or `timeout`; every suite has UTC
  timing and millisecond duration.
  Verify: run a controlled failing suite or helper-level test and inspect
  `failure.type`, `failure.message`, and `timing.duration_ms`.
  Files: `tests/windows/desktop-regression/lib/DesktopRegression.ps1`,
  `tests/windows/desktop-regression/run.ps1`.

- [ ] Task: Document the new artifact contract.
  Acceptance: framework documentation names `desktop-regression.results/v1`,
  explains the result location, and notes that full suites remain manual.
  Verify: run `-List` and compare documented commands with actual runner
  parameters.
  Files: `tests/windows/desktop-regression/README.md`,
  `tests/windows/desktop-regression/SPEC.md`.
