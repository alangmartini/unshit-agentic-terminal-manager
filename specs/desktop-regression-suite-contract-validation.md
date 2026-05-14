# Spec: Desktop Regression Suite Contract And Validation

## Objective

Define a strict suite registration contract for
`tests/windows/desktop-regression` so malformed suites fail during discovery,
before the runner builds the app or controls the desktop. The contract covers
suite names, required metadata, tags, duplicate registration, unsafe characters,
and runner selection by tags.

The users are engineers and agents adding or running manual Windows desktop
regression suites. Success means a bad suite definition produces a clear,
actionable error, and valid existing commands keep working.

## Tech Stack

- Windows PowerShell 5.1.
- `tests/windows/desktop-regression/run.ps1` as the canonical runner.
- `tests/windows/desktop-regression/lib/DesktopRegression.ps1` as the shared
  registration, validation, context, Win32, and assertion library.
- `tests/windows/desktop-regression/suites/*.ps1` as suite definitions that
  call `Register-DesktopRegressionSuite`.
- No new runtime dependency should be required for validation.

## Commands

- List all suites:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -List`
- List suites matching every requested tag:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -List -Tag resize`
- Run suites matching every requested tag:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Tag resize`
- Run explicit suites and then narrow by tag:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Suite edge-resize-stability -Tag input`
- Existing suite selection remains supported:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -Suite post-resize-glitches`
- Registration validation smoke check:
  `powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ". .\tests\windows\desktop-regression\lib\DesktopRegression.ps1; Register-DesktopRegressionSuite -Name 'bad name' -Title 'Bad' -Covers 'Bad metadata.' -Tags @('windows') -ScriptBlock {}"`

## Project Structure

- `tests/windows/desktop-regression/run.ps1`: parses runner filters, discovers
  suites, applies suite/tag selection, lists suites, builds, executes, and
  writes `results.json`.
- `tests/windows/desktop-regression/lib/DesktopRegression.ps1`: owns the suite
  registry and validation helpers used by `Register-DesktopRegressionSuite`.
- `tests/windows/desktop-regression/SPEC.md`: framework-level contract and
  boundaries. Keep it aligned with new validation and tag filtering behavior.
- `tests/windows/desktop-regression/README.md`: user-facing commands, including
  the new `-Tag` examples and metadata requirements.
- `tests/windows/desktop-regression/templates/suite.ps1`: canonical example for
  valid `-Name`, `-Title`, `-Covers`, `-Tags`, and `-ScriptBlock` metadata.
- `tests/windows/desktop-regression/suites/*.ps1`: existing and future suites
  that must pass the contract at dot-source time.

## Code Style

Keep validation small, explicit, and close to registration. Use PowerShell
functions with verb-noun names, `Set-StrictMode -Version Latest`, mandatory
parameters for required metadata, and exact error text that tells the author how
to fix the suite.

```powershell
function Assert-DesktopRegressionKebabCase {
    param(
        [Parameter(Mandatory = $true)][string]$Value,
        [Parameter(Mandatory = $true)][string]$Field,
        [Parameter(Mandatory = $true)][string]$SuiteName
    )

    $pattern = '^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$'
    if ($Value -ne $Value.Trim() -or $Value -notmatch $pattern) {
        throw ("Invalid desktop regression {0} '{1}' for suite '{2}'. " +
            "Use lowercase kebab-case with letters, numbers, and hyphens only.") -f `
            $Field, $Value, $SuiteName
    }
}
```

Key conventions:

- Suite names and tags use lowercase kebab-case.
- Error messages start with the invalid concept: `Invalid desktop regression
  suite name`, `Invalid desktop regression tag`, `Missing desktop regression
  suite metadata`, or `Duplicate desktop regression suite name`.
- Validation errors are terminating errors raised before Win32 initialization,
  build, or suite execution.
- Do not silently sanitize suite names. Names are reused in selection,
  artifacts, and result records, so invalid names must fail.

## Suite Contract

Required registration metadata:

- `Name`: required, non-empty, trimmed, lowercase kebab-case, unique
  case-insensitively.
- `Title`: required, non-empty, trimmed, single-line human-readable text.
- `Covers`: required, non-empty, trimmed, single-line sentence describing the
  protected behavior.
- `Tags`: required, non-empty, trimmed lowercase kebab-case values, unique
  case-insensitively within the suite, and must include `windows`.
- `ScriptBlock`: required and not `$null`.

Suite name validation:

- Allowed format: `^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$`.
- Reject uppercase characters, spaces, underscores, dots, leading hyphens,
  trailing hyphens, repeated path-like separators, wildcards, quotes, backticks,
  colons, semicolons, shell metacharacters, and control characters.
- Reject names that differ only by case from an already registered suite.
- Do not transform invalid names. Authors must fix the suite file.

Tag validation:

- Allowed format matches suite names.
- Tags are normalized only for comparison, not rewritten for storage.
- Duplicate tags on one suite are invalid.
- `windows` is required because this framework is Windows-only and future OS
  desktop frameworks should live in sibling directories.

Clear error messages:

- Missing metadata:
  `Missing desktop regression suite metadata for '<name>': Title, Covers.`
- Invalid suite name:
  `Invalid desktop regression suite name '<value>'. Use lowercase kebab-case with letters, numbers, and hyphens only; do not use spaces, path separators, wildcards, quotes, dots, leading hyphens, or trailing hyphens.`
- Duplicate suite name:
  `Duplicate desktop regression suite name '<name>'. Suite names are case-insensitive and must be unique.`
- Invalid tag:
  `Invalid desktop regression tag '<tag>' for suite '<name>'. Tags must use lowercase kebab-case with letters, numbers, and hyphens only.`
- Duplicate tag:
  `Duplicate desktop regression tag '<tag>' for suite '<name>'. Tags are case-insensitive and must be unique per suite.`
- Missing `windows` tag:
  `Desktop regression suite '<name>' must include the 'windows' tag.`
- Unknown selected suite:
  `Unknown desktop regression suite '<name>'. Known suites: <names>.`
- Unknown selected tag:
  `Unknown desktop regression tag '<tag>'. Known tags: <tags>.`
- Empty filter result:
  `No desktop regression suites matched the requested filters. Suites: <suite filters>; tags: <tag filters>.`

## Runner Filtering

- Add a runner parameter named `-Tag` with type `[string[]]`.
- `-Tag` values use the same validation as suite tags.
- Multiple `-Tag` values use AND semantics: a suite must include every requested
  tag to match. This keeps manual desktop execution focused and avoids broader
  runs than the caller asked for.
- `-Suite` remains exact-name selection and keeps the current clear
  unknown-suite error.
- `-Suite` and `-Tag` can be combined. The runner first resolves explicit suite
  names in registration order, then narrows that set by tags.
- Duplicate `-Suite` or `-Tag` arguments should not execute or list the same
  suite more than once.
- `-List` with no filters remains compatible and lists every registered suite
  with name, title, covers, and tags.
- `-List -Tag <tag>` and `-List -Suite <name>` list the filtered suites and exit
  without Win32 initialization, build, or desktop control.
- Selection and filter validation must happen before `Initialize-DesktopRegressionWin32`,
  before `cargo build`, and before artifact directory creation.

## Testing Strategy

- Use non-invasive discovery checks for most validation:
  `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -List`
- Add focused PowerShell validation tests by dot-sourcing
  `lib/DesktopRegression.ps1` in a fresh process and registering valid and
  invalid suites directly. These tests should not launch `terminal-manager.exe`.
- Cover valid current suites:
  `post-resize-glitches` and `edge-resize-stability` must still register,
  list, and select by `-Suite`.
- Cover validation failures for empty metadata, invalid names, invalid tags,
  duplicate suite names, duplicate tags, missing `windows`, unknown `-Suite`,
  unknown `-Tag`, and empty combined filters.
- Cover filtering behavior with at least:
  `-Tag resize`, `-Tag resize,input`, `-Tag visual`, `-Suite edge-resize-stability -Tag input`,
  and `-Suite edge-resize-stability -Tag visual`.
- Full suite execution remains manual because it moves the mouse, sends keys,
  captures the desktop, and controls native windows.

## Boundaries

- Always: validate suite metadata during registration.
- Always: fail before build or desktop control when registration or filters are
  invalid.
- Always: keep error messages deterministic and useful in terminal output.
- Always: keep existing valid suite names and commands compatible.
- Ask first: changing tag filter semantics from AND to OR.
- Ask first: adding external test dependencies such as Pester.
- Ask first: adding CI execution for desktop suites.
- Never: silently rewrite suite names or tags.
- Never: allow path separators or shell metacharacters in suite names.
- Never: run full desktop suites as part of validation smoke tests unless the
  caller explicitly asks for desktop control.

## Compatibility Behavior

- Current valid suites remain valid without file renames.
- Existing commands without `-Tag` keep their current behavior.
- Existing `-List` output remains structurally compatible; it may be filtered
  only when the caller passes `-Suite` or `-Tag`.
- Historical wrapper scripts that forward to the canonical runner keep working
  because `-Tag` is additive and all existing parameters remain.
- Invalid suite definitions are not compatibility-protected. They should fail
  during discovery with the clear validation errors listed above.
- `results.json` keeps using suite `name`, `title`, `status`, and
  `duration_seconds`. Tag filtering changes which suites run, not the result
  schema.

## Plan

1. Add validation helpers in `lib/DesktopRegression.ps1` for kebab-case values,
   required single-line metadata, duplicate detection, and tag checks.
2. Update `Register-DesktopRegressionSuite` to call validation before adding to
   `$script:DesktopRegressionSuites`.
3. Add `-Tag` parsing and filter resolution in `run.ps1`, keeping selection
   before Win32 initialization and build.
4. Update `README.md`, `SPEC.md`, and `templates/suite.ps1` so the documented
   contract matches runtime behavior.
5. Add non-invasive validation tests or smoke commands that exercise success and
   failure paths without running desktop interactions.

Risks and mitigations:

- Risk: tag filter semantics may surprise callers. Mitigate by documenting AND
  semantics in help text, README, and errors.
- Risk: stricter validation may break local draft suites. Mitigate with
  actionable errors and a template that already follows the contract.
- Risk: validation tests accidentally initialize Win32 or build. Mitigate by
  testing registration directly in a fresh PowerShell process.

Verification checkpoints:

- After registration validation, `run.ps1 -List` still lists both current
  suites.
- After tag filtering, `run.ps1 -List -Tag resize` lists both current suites,
  while `run.ps1 -List -Tag visual` lists only `post-resize-glitches`.
- After docs updates, all examples use valid suite names and tags.

## Tasks

- [ ] Task: Add suite metadata validation helpers.
  - Acceptance: helpers reject invalid names, invalid tags, empty strings,
    multi-line metadata, duplicate tags, and missing `windows`.
  - Verify: run focused PowerShell commands that dot-source
    `lib/DesktopRegression.ps1` and assert expected failures.
  - Files: `tests/windows/desktop-regression/lib/DesktopRegression.ps1`

- [ ] Task: Enforce validation in `Register-DesktopRegressionSuite`.
  - Acceptance: valid current suites register; invalid and duplicate suite
    definitions fail with the specified messages before registry mutation.
  - Verify: `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -List`
  - Files: `tests/windows/desktop-regression/lib/DesktopRegression.ps1`

- [ ] Task: Implement runner tag filtering.
  - Acceptance: `-Tag` supports one or more tags with AND semantics, combines
    with `-Suite`, de-duplicates selected suites, and fails on unknown tags or
    empty matches before build/Win32 initialization.
  - Verify:
    `powershell.exe -ExecutionPolicy Bypass -File tests\windows\desktop-regression\run.ps1 -List -Tag resize`
  - Files: `tests/windows/desktop-regression/run.ps1`

- [ ] Task: Document the suite contract and examples.
  - Acceptance: README, framework spec, and template all describe required
    metadata, lowercase kebab-case names/tags, `windows` tag, and `-Tag` usage.
  - Verify: manually compare docs against the runtime validation messages.
  - Files: `tests/windows/desktop-regression/README.md`,
    `tests/windows/desktop-regression/SPEC.md`,
    `tests/windows/desktop-regression/templates/suite.ps1`

- [ ] Task: Add non-invasive validation coverage.
  - Acceptance: validation and filtering behavior can be checked without
    launching the desktop app, moving input, or capturing screenshots.
  - Verify: run the new validation test command or script plus
    `run.ps1 -List`.
  - Files: a focused validation test file or existing test harness location
    chosen during implementation

## Success Criteria

- Invalid suite names, unsafe characters, duplicate suite names, missing
  metadata, invalid tags, duplicate tags, and missing `windows` tags fail during
  registration with the specified clear errors.
- `run.ps1 -List` and existing `run.ps1 -Suite <name>` behavior remain
  compatible for the current suites.
- `run.ps1 -List -Tag resize` and `run.ps1 -Tag resize` select suites by tag
  without requiring callers to know every suite name.
- Selection errors happen before build, artifact directory creation, Win32
  initialization, or desktop control.
- Documentation and template examples match the enforced contract.
- Validation coverage does not require full manual desktop suite execution.

## Open Questions

- Should tag filtering stay AND-only, or should a future `-AnyTag`/`-AllTags`
  distinction be added if OR semantics are needed?
- Should there be an approved tag vocabulary beyond requiring `windows` and
  lowercase kebab-case?
- Should validation tests use plain PowerShell assertions to avoid dependencies,
  or is adding Pester acceptable for this repository?
