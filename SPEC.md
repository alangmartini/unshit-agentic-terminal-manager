# SPEC: Split Panel Functionality Enhancement

## 1. Objective

Extend the existing split pane system so a user can:

1. Drag one tab into another tab's pane, see a live preview of where the drop will land (5 zones: top / right / bottom / left edges plus center), and drop to create a split or merge as a stacked tab.
2. Split the currently focused pane vertically or horizontally via hotkey.
3. Undo a split by closing the focused pane (or dragging its header out).
4. Drag a split pane's header up to the tab bar to extract it back into a standalone tab.
5. Edit the hotkeys used for all of the above from Settings > Keybinds, with click-to-record, conflict detection, per-row + global reset, and persistence to a JSON config.

Target user: the repo owner (Alan), running the terminal-manager app locally on Windows. No multi-user or remote scenarios.

### What's already there (do NOT rebuild)

* 2D grid pane model in `src/state.rs` with `row_ratios` and `col_ratios` per tab.
* Split rendering via `src/ui/terminal_grid.rs` with working column and row drag resizers.
* Dispatch commands `pane.split_right`, `pane.split_down`, `pane.close`.
* Read-only keybinds list in `src/ui/settings.rs` under `SettingsSection::Keybinds`.
* Framework `DragEvent` support with Start / Update / End phases.

### What this spec adds

* Drop-zone overlay and hit-testing (5 zones per pane).
* Tab drag lifecycle (pick up, hover, drop -> split or merge).
* Pane header with drag handle; drag out to tab bar extracts pane to new tab.
* Keybind registry wired to actual dispatch (currently constants are displayed but some hotkeys are not bound to the registry).
* Editable keybind UI: record combos, detect conflicts, reset, persist to JSON.
* Hotkey for "unsplit" (close focused pane; if last pane, close tab).

## 2. Core Features and Acceptance Criteria

### F1: Tab drag into pane with 5-zone drop preview

**User story:** I grab tab B by its label, hover over tab A's pane area, see a translucent overlay split into 5 zones (top / right / bottom / left / center). As my cursor moves, the zone under it highlights. Dropping on an edge zone creates a split on that edge; dropping on center merges B into A as a stacked tab group.

**Acceptance:**
* Dragging a tab more than 4px (existing drag threshold) enters tab-drag mode; the original tab shows a "ghost" placeholder.
* While dragging, a full-viewport overlay renders above all panes with per-pane 5-zone hit regions. Edge zones occupy the outer 25% on each side; center occupies the inner region.
* Hovered zone highlights with a translucent accent fill.
* Drop on edge zone calls a new dispatch `pane.drop_split:{target_pane_id}:{edge}` where edge is `top|right|bottom|left`. The source terminal moves from its tab into the target tab at the new split position.
* Drop on center calls `tab.merge:{target_tab_id}:{source_tab_id}` (if we adopt tab stacking) OR just moves the source tab adjacent to the target (if we skip stacking for v1).
* Drop outside any zone (on the tab bar gap or outside the window) cancels; source tab returns to its original position.
* If the source tab is the only tab in its workspace and the drop would leave that workspace empty, the source tab is moved rather than destroyed.

**Scope decision needed:** v1 treats "center" as "reorder source tab next to target tab" and defers true tab stacking. Flagged in Boundaries.

### F2: Split hotkeys

**User story:** I press a hotkey and my focused pane splits vertically or horizontally, with a new shell spawned in the new pane.

**Acceptance:**
* Default bindings: `Ctrl+Shift+V` = split vertical (right), `Ctrl+Shift+H` = split horizontal (down). Existing `Ctrl+D` / `Ctrl+Shift+D` remain aliases.
* Hotkey fires the existing `pane.split_right` / `pane.split_down` dispatch.
* New pane gets focus and spawns the default shell in the workspace's cwd.
* Keybinds are registered in the `ShortcutRegistry` and dispatch through it (not hard-coded match arms).

### F3: Unsplit hotkey

**User story:** I press a hotkey and my focused pane closes. If it was the last pane, the tab closes.

**Acceptance:**
* Default binding: `Ctrl+Shift+W`.
* Fires `pane.close` on the focused pane.
* If the closed pane was the only pane in the tab, the tab closes (same as `Ctrl+W`).
* PTY for the closed pane is killed and cleaned up.

### F4: Drag pane to tab bar to extract

**User story:** I grab a split pane's header bar and drag it upward onto the tab bar area. A tab-shaped placeholder appears at the drop position. Releasing creates a new tab containing only that pane's terminal; the source tab reflows.

**Acceptance:**
* Every split pane (panes in a tab with >1 pane) has a 24px header bar at the top showing the terminal's title and a drag grip.
* Header bar is the drag source (not the terminal body, to avoid conflicts with text selection).
* When a single pane exists in a tab, the header is hidden (there's only one pane; no need to show it).
* Dragging the header more than 4px enters pane-drag mode.
* Over the tab bar, a tab-sized placeholder indicates insertion point between existing tabs.
* Drop on tab bar dispatches `pane.extract_to_tab:{source_pane_id}:{insert_index}`. The source pane's terminal and its PTY move to a new tab; the source tab's layout recomputes (remaining panes reflow with normalized ratios).
* Dropping outside the tab bar while pane-dragging falls back to F1 semantics (drop into another pane as a split).

### F5: Editable keybinds in Settings

**User story:** I open Settings > Keybinds. I see a row per action with its current combo. I click a combo, the row enters "recording" mode, I press a new combo, and it saves. If the combo conflicts with another action's binding, I see a red warning and cannot save until resolved. I can reset one row or all rows to defaults.

**Acceptance:**
* Settings > Keybinds shows a table: `Action | Combo | Reset | (conflict indicator)`.
* Clicking the combo cell starts recording: the cell shows "Press keys..." and intercepts the next key combo.
* Pressing `Escape` cancels recording; pressing any other combo captures modifier mask + key.
* On capture, the UI checks the in-memory keybinds map. If another action has the same combo, both rows show a red border and a conflict banner appears; the save is rejected and the original combo is kept.
* "Reset" per row restores that action's default combo.
* "Reset all" button at the top of the section restores every action.
* On every successful change, the keybinds map is serialized to `<app_config_dir>/keybindings.json` and the `ShortcutRegistry` is updated live (next keypress uses the new combo; no restart needed).
* On app start, `keybindings.json` is loaded if present; missing entries fall back to defaults.

## 3. Commands (app-level dispatch strings)

New commands to add to `dispatch()` in `src/state.rs`:

```
pane.drop_split:<target_pane_id>:<edge>        // edge = top|right|bottom|left
pane.extract_to_tab:<source_pane_id>:<index>   // index = tab bar insertion index
tab.reorder:<source_tab_id>:<index>            // center-zone drop / tab bar drop between tabs
keybind.set:<action>:<combo>                   // action = snake_case id, combo = "ctrl+shift+v"
keybind.reset:<action>                          // reset one action to default
keybind.reset_all                               // reset all to defaults
```

Existing commands reused: `pane.split_right`, `pane.split_down`, `pane.close`, `tab.new`, `tab.close`, `tab.switch`.

## 4. Project Structure

New files:

```
src/drag/
    mod.rs              // Drag state machine: Idle, DraggingTab, DraggingPane
    drop_zones.rs       // 5-zone hit-testing: compute zone for (pane_rect, cursor)
    overlay.rs          // Render translucent drop-zone overlay + highlight

src/ui/pane_header.rs   // 24px header bar with title and drag grip

src/keybinds/
    mod.rs              // KeybindAction enum, KeybindMap, serialization
    defaults.rs         // Default combos for every action
    loader.rs           // Load/save keybindings.json from app_config_dir
    recorder.rs         // "Record a combo" capture state for the UI
```

Modified files:

```
src/state.rs            // Add new dispatch arms; add ExtractToTab / DropSplit mutations
src/ui/tabbar.rs        // Make tabs draggable; render drop placeholder
src/ui/terminal_grid.rs // Wrap each pane in header + drop-zone participant
src/ui/settings.rs      // Replace static Keybinds section with editable table
src/bridge.rs           // Subscribe to keybind registry changes; update live
crates/unshit-framework/crates/unshit-app/src/shortcut.rs  // Only if registry needs API for live updates
```

## 5. Code Style

Follow existing repo conventions:

* Rust 2021 edition, `cargo fmt` default settings, `cargo clippy` clean.
* Module files prefer `mod.rs` + submodules when the namespace has >2 files (matches existing `src/ui/`, `src/pty/`).
* Dispatch commands use `snake.case_colon_args` format (matches existing `pane.split_right`, `tab.switch:3`).
* UI builders return `Element` via unshit framework primitives; no direct HTML strings.
* No `unwrap()` in non-test code; use `?` or explicit error handling.
* Tests live in `#[cfg(test)] mod tests` inside the module file for unit tests, in `tests/` at the crate root for integration tests.
* Changelog fragment added under `changelog/unreleased/` per repo rule (format per `changelog/TEMPLATE.md`).
* Commit style: conventional (`feat:`, `fix:`, `refactor:`). Atomic commits per feature (F1..F5 can land as separate PRs).

## 6. Testing Strategy

Per repo CLAUDE.md: every feature MUST have coverage. TDD is mandatory (Red -> Green -> Refactor).

### Unit tests (in-module)

* `src/drag/drop_zones.rs`: given a pane rect and cursor position, returns the correct zone. Cover all 5 zones and boundary cases.
* `src/keybinds/mod.rs`: parse/serialize combo strings round-trip; conflict detection returns the correct conflicting action.
* `src/keybinds/loader.rs`: load missing file falls back to defaults; load corrupt file falls back with a logged warning.
* `src/state.rs`: new dispatch arms:
  * `pane.drop_split` on each edge produces the expected `panes` / `col_ratios` / `row_ratios`.
  * `pane.extract_to_tab` moves the correct pane, reflows the source, and sets the new tab active.
  * `pane.close` on last pane closes the tab.
* `src/ui/pane_header.rs`: header is hidden when `panes.len() == 1`, shown otherwise.

### Integration tests (`tests/`)

* `tests/tab_drag_drop.rs`: simulate drag lifecycle via framework test harness; verify overlay render, zone highlight, final layout after drop.
* `tests/pane_extract.rs`: drag pane header to tab bar index N; verify new tab count, active tab, and remaining panes.
* `tests/keybind_roundtrip.rs`: edit a binding via the settings UI (simulated click + key events); verify JSON file updated and `ShortcutRegistry` serves the new combo on next keypress.

### Regression tests

Every bug fix filed during implementation gets a test that reproduces the bug and references the issue number. Required per CLAUDE.md.

### Visual verification

Per existing feedback memory: the user does visual verification himself. Automated tests do NOT take screenshots, move the cursor, or steal foreground. After each feature lands, run `cargo run` and flag the feature as ready for the user to verify visually. Don't claim "it works" without that step.

### Coverage gate

`cargo llvm-cov` must not decrease overall coverage after each PR. Aim for 90%+ on new modules.

## 7. Boundaries

### Always do

* Spawn PTY eagerly on new panes/tabs (80x24 initial) per CLAUDE.md pitfall #4.
* Keep the existing 2D grid model. Do NOT refactor to a binary tree; extend the flat model with the new mutations.
* Write failing tests first (TDD) for every feature slice.
* Run `cargo test`, `cargo clippy`, `cargo fmt --check` before every commit.
* Add a changelog fragment under `changelog/unreleased/` for every `feat:` / `fix:` PR.
* Use the git-workflow-manager agent for commits and PRs per user's global rule.
* Land F1..F5 as separate PRs (atomic commits) unless the user says otherwise.

### Ask first

* Whether to build true tab stacking (multiple terminals sharing one tab slot) for the F1 "center" zone, or ship v1 with "reorder adjacent" semantics only. Default: reorder adjacent.
* Whether extracting the last pane of a tab should close the source tab or keep it with a fresh shell. Current answer: close (matches F3 behavior).
* Before touching the framework code in `crates/unshit-framework/` for anything beyond subscribing to existing APIs. Prefer solving in app code.

### Never do

* Do NOT add keybind hot-reload via file watcher in v1. Changes are pushed from the settings UI directly to the registry; the JSON file is a persistence sink, not a live input.
* Do NOT refactor existing split rendering or ratio math. Those tests are the canonical ground truth; breaking them means the refactor is wrong.
* Do NOT add tab groups, tab pinning, or session save/restore for drag state. Out of scope.
* Do NOT run the visual loop (screenshots, cursor automation, PrintWindow) unless the user asks explicitly.
* Do NOT skip pre-commit hooks (`--no-verify`). If a hook fails, fix the cause.

## 8. Out of Scope (v1)

* True tab stacking (stacked terminals per tab slot).
* Session save/restore of drag or split state across app restarts (panes already persist per-tab in state; drag is transient).
* Multi-window drag (drag a pane out into a new OS window).
* Keybind sets / profiles (vim-style, emacs-style presets).
* Import/export of keybindings.json via UI (user can hand-edit the file).

## 9. Implementation Order (suggested)

F2 and F3 are cheapest and unblock the others (hotkey wiring + unsplit). F5 can proceed in parallel with F1 because it touches different files.

1. **F2** Split hotkeys wired to registry.
2. **F3** Unsplit hotkey.
3. **F5** Editable keybinds (unblocks user remapping everything else).
4. **F4** Pane header + extract-to-tab drag.
5. **F1** Full tab drag with 5-zone drop preview (reuses F4's drag machinery).

## 10. Open Questions (to resolve during implementation)

* Does the unshit framework's `DragEvent` carry a pointer position in viewport coordinates? If not, we need a small framework addition to support cross-pane hit-testing in F1.
* Where is `app_config_dir` currently resolved? (Likely `dirs::config_dir()`.) Confirm before writing `keybindings.json`.
* Does the PTY cleanup path handle "move PTY between tabs" without restarting the shell? For F4 we need to hand off the PTY, not kill and respawn.
