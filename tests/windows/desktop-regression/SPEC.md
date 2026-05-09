# Desktop Interaction Regression Framework Spec

## Objective

Provide a manual Windows desktop test framework for `terminal-manager` behaviors
that require a real native window, compositor behavior, global input, and screen
capture. The framework should make these tests discoverable under `tests/`
without adding them to CI/CD yet.

## Target Users

- Engineers validating native window behavior before or after a UI/rendering
  change.
- Agents investigating regressions that cannot be reproduced through unit tests
  or the in-process `unshit-test` harness.

## Commands

- List suites:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -List`
- Run all suites:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1`
- Run one suite:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Suite <name>`
- Run against an existing binary:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -SkipBuild`

Compatibility wrappers in `scripts/` may forward to the canonical runner, but
new documentation should point to `tests/windows/desktop-regression`.

## Project Structure

- `run.ps1`: command-line runner, suite discovery, build step, result writing.
- `lib/DesktopRegression.ps1`: shared Win32, process, screenshot, input, and
  assertion helpers.
- `suites/*.ps1`: scenario files registered with `Register-DesktopRegressionSuite`.
- `templates/suite.ps1`: starting template for new suites.
- `artifacts/windows/desktop-regression/<run_id>/`: ignored output directory containing
  screenshots and `results.json`.

The `tests/windows/` namespace is reserved for Windows-only desktop tests.
Future macOS and Linux desktop frameworks should use sibling OS directories
instead of sharing this runner or helper library.

## Code Style

- Keep suite names behavior-oriented, not issue-number-oriented.
- Each suite should launch and stop its own app session in a `try/finally`.
- Assertions must include clear failure messages that explain the protected
  behavior.
- Prefer helpers in `lib/DesktopRegression.ps1` over ad hoc Win32 calls inside
  suites.
- Keep test artifacts deterministic and grouped by suite/run id.

## Testing Strategy

- Use `-List` as a non-invasive smoke check for suite discovery.
- Use compatibility wrapper `-List` checks to protect old entry points.
- Full suite execution is manual because it takes over real desktop input,
  sends keys such as `Win+Left`, moves the mouse, and captures the screen.

## Boundaries

- Do not add these suites to CI/CD until explicitly requested.
- Do not modify `.github/workflows` for this framework move.
- Do not run full desktop suites without making it clear they will control the
  local desktop.
- Do not reuse this framework for in-process Rust UI assertions; those belong in
  `unshit-test`.
