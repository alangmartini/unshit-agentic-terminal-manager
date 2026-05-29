# Implementation Plan: Terminal Theme Setting

## Overview
Finish the existing partial theme-setting work by making the theme catalog authoritative, preserving the original Amber palette, adding sourced palette families, and applying the selected palette to cloned terminal grids during render.

## Architecture Decisions
- Keep theme application app-level because it maps product-specific terminal palette choices onto existing `CellGrid` snapshots.
- Use render-time grid cloning/mapping so existing terminal sessions update immediately without mutating daemon-owned sessions or blocking PTY writes.
- Keep CSS variables for app shell colors and Rust palette constants for terminal cell colors, with tests ensuring ids stay aligned.
- Keep Amber as the default product palette, and add established palettes: Catppuccin Mocha, Tokyo Night, Nord, Dracula, Everforest, and Rose Pine Moon.

## Task List

### Phase 1: Foundation
- [x] Task 1: Add spec and task docs.
  - Acceptance: Spec covers objective, commands, structure, style, tests, boundaries, and success criteria.
  - Verify: Inspect `specs/themes-setting.md` and `tasks/themes-setting.md`.
  - Files: `specs/themes-setting.md`, `tasks/themes-setting.md`.

- [x] Task 2: Add failing tests for catalog and terminal palette mapping.
  - Acceptance: Tests require concrete sourced themes plus render-time cell color mapping.
  - Verify: `cargo test theme` fails before implementation.
  - Files: `src/theme.rs`, `src/main.rs`.

### Phase 2: Core Feature
- [x] Task 3: Implement theme catalog and render-time terminal color mapping.
  - Acceptance: Theme ids resolve safely; default/ANSI terminal colors map to the active theme; unknown ids fallback.
  - Verify: `cargo test theme`.
  - Files: `src/theme.rs`, `src/main.rs`.

- [x] Task 4: Update settings picker and CSS themes.
  - Acceptance: Appearance settings render all theme examples; click updates active state; CSS has a class for every theme id.
  - Verify: `cargo test settings_page_appearance theme`.
  - Files: `src/ui/settings.rs`, `assets/styles.css`, `assets/themes.json`.

### Phase 3: Quality
- [x] Task 5: Run quality gates and review.
  - Acceptance: Format, focused tests, clippy/full test where practical; review finds no blocking issues.
  - Verify: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`.
  - Files: No additional feature files expected.

## Risks and Mitigations
| Risk | Impact | Mitigation |
|------|--------|------------|
| Theme mapping alters truecolor app output | Medium | Only map canonical default and ANSI colors; preserve high 256-color and truecolor values. |
| Windows Terminal parity tests drift | Medium | Skip app theme terminal mapping when the parity env override is enabled. |
| CSS and Rust theme ids diverge | Medium | Add tests that assert stylesheet classes exist for all catalog theme ids. |

## Open Questions
- Persisting the selected theme can be a follow-up unless existing persistence already exposes a theme field.

## Launch Notes
- Validation passed: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test theme`, `cargo test settings_page_appearance`, `cargo test -p unshit-test --test scroll`, `cargo test`, `cargo test -p unshit-core`, `cargo test -p unshit-test`, and `git diff --check`.
- Manual GUI launch was not run because it opens the interactive app; the feature is covered by unit, UI tree, and framework scroll tests.
- Rollback scope covers the theme feature files plus framework scroll/reconcile styling support touched for immediate theme repaint and scroll repaint. No data migrations or new dependencies were added.
