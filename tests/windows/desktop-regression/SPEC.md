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
  `cargo xtask desktop-regression --list`
- Run all suites:
  `cargo xtask desktop-regression`
- Run one suite:
  `cargo xtask desktop-regression --suite <name>`
- Run against an existing binary:
  `cargo xtask desktop-regression --suite <name> --skip-build --exe-path target\debug\terminal-manager.exe`
- Select observability:
  `cargo xtask desktop-regression --suite <name> --observe off|basic|full`
- Override artifact root:
  `cargo xtask desktop-regression --artifact-root artifacts/windows/desktop-regression`

`--observe off` is black-box and must not enable the app diagnostic endpoint.
`--observe basic` enables runner/app diagnostic capture for handshake, logs,
events, and failure evidence. `--observe full` records step snapshots,
invariants, and cross-layer assertions where the suite supports them.

`tests/windows/desktop-regression/run.ps1` and compatibility wrappers in
`scripts/` forward to the Rust command. The PowerShell wrapper preserves
`-List`, repeated `-Suite`, `-SkipBuild`, `-ExePath`, explicit `-ArtifactsDir`,
and Rust-facing observe/interactive/record flags. Legacy PowerShell-only
threshold and fixture flags are accepted but ignored with warnings because the
migrated Rust suites own those defaults. No legacy-only PowerShell suites remain
for the current suite set.

## Project Structure

- `xtask/src/desktop_regression/`: canonical Rust runner, suite registry,
  build/launch orchestration, Win32 driver, screenshots, diagnostics,
  artifacts, failure bundles, and result writing.
- `run.ps1`: PowerShell compatibility wrapper for old commands.
- `lib/DesktopRegression.ps1`: historical shared Win32, process, screenshot,
  input, and assertion helpers.
- `suites/*.ps1`: historical scenario files retained for compatibility
  reference.
- `templates/suite.ps1`: historical PowerShell starting template.
- `artifacts/windows/desktop-regression/<run_id>/`: ignored output directory
  containing screenshots, logs, diagnostic evidence, failure manifests, and
  `results.json`.

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

- Use `cargo xtask desktop-regression --list` as a non-invasive smoke check for
  suite discovery.
- Use PowerShell compatibility wrapper `-List` checks to protect old entry
  points.
- Use one-suite `--skip-build --exe-path <path>` checks when a built binary is
  available and a full headed run is acceptable.
- Full suite execution is manual because it takes over real desktop input,
  sends keys such as `Win+Left`, moves the mouse, and captures the screen.

## Boundaries

- Do not add these suites to CI/CD until explicitly requested.
- Do not modify `.github/workflows` for this framework move.
- Do not run full desktop suites without making it clear they will control the
  local desktop.
- Do not reuse this framework for in-process Rust UI assertions; those belong in
  `unshit-test`.
