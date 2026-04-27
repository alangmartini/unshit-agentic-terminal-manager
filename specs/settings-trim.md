# SPEC: Trim unimplemented Settings options

## 1. Objective

The Settings modal in `src/ui/settings.rs` advertises a long list of options that look configurable but do nothing. Only Font size, Keybinds, Sessions, and Danger Zone actions actually drive behavior. Everything else is one of:

* A control with no `on_click` handler (cursor style, opacity slider, line-height stepper, agent default-timeout stepper).
* A static read-only display masquerading as an input (default shell, working directory, history size, word separators).
* A toggle whose value is stored in `state.toggles` and never read by anything except the toggle itself (restore-on-startup, confirm-close, start-minimized, check-updates, glow-effect, background-texture, font-ligatures, shell-integration, scroll-on-output, bell-notification, auto-discovery).
* A hardcoded fake list (the three agent rows: claude / amp / codex).

This spec defines the cleanup: rip every non-functional row out of the settings UI, drop the corresponding backend plumbing, collapse the now-empty sections, and keep tests honest. The result is a Settings modal where every visible control changes something observable.

**Target user:** the repo owner (Alan), running terminal-manager locally on Windows.

**Non-goals:**

* Implementing any of the trimmed features (themes, opacity, ligatures, etc.). If we want them later, they come back as new work with real wiring.
* Reworking the modal's visual style, footer buttons, or nav chrome beyond removing entries.
* Touching the close-app dialog flow that consumes `RememberCloseChoice` / `KillAllOnClose`.

## 2. Core Features and Acceptance Criteria

### F1: Trim dead rows from Appearance section

**Drop:** `Cursor style`, `Terminal opacity`, `Line height`, `Glow effect`, `Background texture`, `Font ligatures`.

**Keep:** `Theme` (chips visibly highlight on click, accepted scope from clarifying questions), `Font size`.

**Acceptance:**

* `build_appearance_section` produces exactly two rows: theme chip group and font stepper.
* `cursor_style_group`, `slider_control`, and the `stepper(value, None)` call site for line-height are removed (no callers remain).
* No `ToggleKey::GlowEffect`, `BackgroundTexture`, or `FontLigatures` usages remain in the file.

### F2: Remove the General section entirely

**Acceptance:**

* `SettingsSection::General` variant is removed from the enum, and `SettingsSection::all()` no longer yields it.
* `build_general_section` and its row helpers (`select_display`, `text_input_display`) are deleted if no other section uses them. `compact_input_display` goes too once the Shell section drops.
* Default settings section becomes `Appearance` (verified by `seed_state` and tests).
* The nav strip no longer renders a "general" entry.

### F3: Remove the Shell section entirely

**Acceptance:**

* `SettingsSection::Shell` removed from the enum and `all()`.
* `build_shell_section` deleted.
* `ToggleKey::ShellIntegration`, `ScrollOnOutput`, `BellNotification` removed from the enum, the `as_str` match, and the `seed_state` defaults map.

### F4: Remove the Agents section entirely

**Acceptance:**

* `SettingsSection::Agents` removed from the enum and `all()`.
* `build_agents_section`, `AGENT_SPECS`, `AgentSpec`, `AgentStatus`, `agent_list_header`, `agent_row`, `agent_toggle_button`, `agent_badge` deleted from `src/ui/settings.rs`.
* `AgentKey` enum, `Agent` struct, `agent_enabled`, `mutate_toggle_agent`, and the `agents: Vec<Agent>` field on `AppState` and `UiSnapshot` are deleted from `src/state.rs`.
* `ToggleKey::AutoDiscovery` removed from the enum, `as_str`, and seed defaults.
* `icon_agent` import in `settings.rs` is removed if no other section uses it (verify via grep).

### F5: Trim dead General-section toggles from `ToggleKey`

**Acceptance:**

* `ToggleKey::RestoreOnStartup`, `ConfirmClose`, `StartMinimized`, `CheckUpdates` are removed from the enum, the `as_str` match, the `seed_state` defaults map, and any test fixtures that reference them.
* `RememberCloseChoice` and `KillAllOnClose` remain untouched: they drive the close-app flow and are persisted via `persist.rs`.

### F6: Update the settings nav

**Acceptance:**

* After F2 / F3 / F4, the nav contains exactly four items in this order: `Appearance`, `Keybinds`, `Sessions`, `Danger Zone`.
* The default `settings_section` set in `seed_state` (`General`) is updated to `Appearance` so the modal opens to a non-empty section.

### F7: Tests reflect the new shape

**Acceptance:**

* Existing assertions that count rows or assert section presence (search for `appearance_section_*`, `build_general_section`, `build_shell_section`, agent_* tests) are updated to match the trimmed structure or deleted if they cover removed code.
* No test references a removed `ToggleKey` variant or a removed `AgentKey`.
* Persistence round-trip test in `persist.rs` still passes (it only touches `RememberCloseChoice` and `KillAllOnClose`).

### F8: Build and runtime checks

**Acceptance:**

* `cargo build` and `cargo test` pass with zero warnings.
* `cargo clippy --all-targets -- -D warnings` passes.
* `cargo run` opens the app, the Settings modal renders the four nav items, every visible row reacts to interaction or scrolls without dead controls.

## 3. Tech Stack and Constraints

* Rust 2021 edition. App built on the in-tree `unshit` framework at `crates/unshit-framework/`.
* No new dependencies. No new files except this spec.
* All UI in `src/ui/settings.rs`; state in `src/state.rs`; persistence in `src/persist.rs`; close-app wiring in `src/main.rs`.
* CSS at `assets/styles.css` may shed unused class blocks (e.g. `.cursor-group`, `.slider`, `.agent-row`, `.theme-chip` survives) but only after grep confirms zero usages remain.

## 4. Project Structure Impact

```
src/
  state.rs        edits: shrink ToggleKey enum, drop AgentKey/Agent/agents,
                  change default settings_section to Appearance, prune seed_state.
  persist.rs      no edits expected (still reads only the two close toggles).
  ui/settings.rs  edits: drop General/Shell/Agents builders, prune Appearance,
                  drop now-unused helpers + AGENT_SPECS, update tests.
  main.rs         no edits expected (only references the two close toggles).
assets/
  styles.css      edits: remove unused class blocks for trimmed controls.
specs/
  settings-trim.md  this file (new).
```

## 5. Code Style

* Follow existing conventions in `src/ui/settings.rs` (no comments unless explaining a non-obvious why; element builders chained with `.with_*`).
* Removed code is **deleted, not commented out** (per `CLAUDE.md`: no `// removed` placeholders, no `_unused_var` renames).
* `cargo fmt` clean before commit.
* Conventional commit prefix: `refactor:` (purely removing dead UI/code, no new behavior). Issue/PR linkage: open a GitHub issue for "Trim unimplemented settings options" and reference it as `fixes #N`.

## 6. Testing Strategy

* **Red phase first.** Before removing each section, write or update a test that asserts the new structure (e.g. `settings_nav_has_four_items`, `appearance_section_has_two_rows`). It should fail against the current code.
* **Green phase.** Apply the trim until the new tests pass.
* **Regression coverage.** Keep persistence round-trip and close-flow tests intact; confirm they still pass without changes.
* **No mocking of toggle storage** — tests use `seed_state()` like existing tests.
* `cargo llvm-cov` should not show coverage drops on remaining code paths (close-flow, font stepper, keybinds, sessions, kill-all).
* Manual verification: `cargo run`, open Settings, walk through every nav item, confirm every row in the visible sections still does something.

## 7. Boundaries

**Always do:**

* Delete dead enum variants, dead struct fields, and dead helpers in the same change set as the UI rows that referenced them. No orphan symbols.
* Run `cargo build && cargo test && cargo clippy && cargo fmt --check` before commit.
* Verify the app visually via `cargo run` after the refactor (per `CLAUDE.md` guidance that tests don't catch UI regressions).

**Ask first:**

* Before deleting `theme` field from `AppState` or any theme-related code (kept in scope per clarifying-question answer).
* Before touching anything outside `src/ui/settings.rs`, `src/state.rs`, `assets/styles.css`, and tests in those files.
* Before reworking the modal footer (`save changes` / `cancel` buttons) — they're cosmetic but out of scope.
* Before changing the close-app dialog or its toggles.

**Never do:**

* Comment out dead code instead of deleting it.
* Rename unused identifiers with a leading underscore as a workaround.
* Introduce new abstractions or helpers "for the trimmed-down version" — simplify by deletion.
* Touch `crates/unshit-framework/` for this refactor; nothing here belongs upstream.
* Skip pre-commit hooks (`--no-verify`) or commit signing.

## 8. Out of Scope (Explicit)

* Re-implementing any of the removed features.
* Rewriting the Settings modal layout or footer behavior.
* Persisting `theme` or `font_size_pt` (separate work if desired).
* Anything in the Keybinds, Sessions, or Danger Zone sections.
