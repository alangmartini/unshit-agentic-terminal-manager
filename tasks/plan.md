# Plan: Split Panel Functionality

Derived from `SPEC.md`. Order: F2 -> F3 -> F5 -> F4 -> F1. Each task is a vertical slice: data + dispatch + UI + tests, landed as one PR.

## Ground truth from code exploration

* Dispatch lives in `src/state.rs` `dispatch()` at ~line 1042 with no `keybind.*`, `pane.drop_split`, `pane.extract_to_tab`, or `tab.reorder` arms yet.
* `ShortcutRegistry` (`crates/unshit-framework/crates/unshit-app/src/shortcut.rs` lines 157 onward) already supports runtime `register` and `unregister` with `BindingPriority`. No framework change needed for F5's hot-reload.
* `DragEvent` carries absolute viewport `x`, `y` plus deltas. F1's open question (cross-pane hit-testing) is resolved; no framework addition required.
* `build_pane_header()` already exists at `src/ui/terminal_grid.rs` ~line 201 as display-only. F4 adds drag to it; also hides it when `panes.len() == 1`.
* Today's read-only keybinds in `src/ui/settings.rs` lines 230 to 246: `New terminal`, `Close tab`, `Split right`, `Split down`, `Next tab`, `Previous tab`, `Command palette`, `Toggle sidebar`, `Settings`, `Zoom in`, `Zoom out`, `Fullscreen`.
* No `changelog/` directory exists in this repo. Skip changelog fragments (user's rule is "if it exists").

## Dependency graph

```
F2 (split hotkeys)  ----\
                         +--- F5 (editable keybinds)
F3 (unsplit hotkey) ----/           |
                                    v
                         F4 (pane extract to tab)
                                    |
                                    v
                         F1 (tab drag + 5-zone drop)
```

F2 and F3 are independent and can land in either order. F5 consumes the wired-up keybind registry from F2/F3. F4 introduces the pane-header drag-source and `pane.extract_to_tab` dispatch that F1 reuses. F1 is last because it stacks on everything.

## Phases and checkpoints

### Phase A: Hotkey plumbing (F2 + F3)

**Goal:** every split/unsplit action is a named action in a registry, not a hard-coded match arm. Unblocks F5.

---

#### Task A1: Introduce `KeybindAction` enum and action -> default-combo map

**Files**: `src/keybinds/mod.rs` (new), `src/keybinds/defaults.rs` (new), `src/state.rs` (import).

**Acceptance:**
* `KeybindAction` enum has one variant per dispatched action currently shown in Settings: `NewTerminal`, `CloseTab`, `SplitRight`, `SplitDown`, `Unsplit`, `NextTab`, `PrevTab`, `CommandPalette`, `ToggleSidebar`, `OpenSettings`, `ZoomIn`, `ZoomOut`, `Fullscreen`.
* `KeybindAction::dispatch_command(self) -> &'static str` returns the dispatch string (e.g. `SplitRight -> "pane.split_right"`).
* `KeybindAction::default_combo(self) -> KeyCombo` returns the default combo parsed via `KeyCombo::parse`.
* `KeybindAction::label(self) -> &'static str` returns the human label.
* `KeybindAction::id(self) -> &'static str` returns a stable snake_case id (used for JSON serialization).

**Verification:**
* Unit tests in `src/keybinds/mod.rs` cover `id()` round-trips, `default_combo()` parses successfully for every variant, `dispatch_command()` matches the strings in `src/state.rs dispatch()`.
* `cargo test keybinds::` passes.
* `cargo clippy` clean.

**Red -> Green -> Refactor:** Write the parse/round-trip tests first. Add enum + functions until they pass.

---

#### Task A2: Register default bindings and dispatch through `ShortcutRegistry`

**Files**: `src/main.rs` or `src/bridge.rs` (wherever `ShortcutResolver` is constructed), `src/keybinds/registry.rs` (new helper).

**Acceptance:**
* On app start, every `KeybindAction` default is registered in `ShortcutResolver` with its dispatch command and `BindingPriority::Default`.
* Key events resolved through `resolver.process_key(...)` produce the dispatch string, which is routed to `state::dispatch()`.
* Existing hard-coded keybind handling that bypasses the registry (if any) is removed in favor of the registry path.
* `Ctrl+Shift+V` and `Ctrl+Shift+H` are registered as aliases for `SplitRight` and `SplitDown` per spec F2.
* `Ctrl+Shift+W` is registered for the new `Unsplit` action per spec F3.

**Verification:**
* Integration test simulating key events (via the existing shortcut test infrastructure) confirms each combo fires the correct dispatch string.
* `cargo test` passes.
* `cargo run` and visually verify: `Ctrl+Shift+V` splits right; `Ctrl+Shift+H` splits down; `Ctrl+Shift+W` closes the focused pane. User does visual verification per memory.

---

#### Task A3: Implement `Unsplit` dispatch with last-pane-closes-tab semantics

**Files**: `src/state.rs`.

**Acceptance:**
* New dispatch arm `pane.close` (already exists) is updated so that when the closed pane is the tab's only pane, the tab is also closed.
* If this behavior already exists in `pane.close`, add a unit test asserting it and move on.
* The associated PTY is killed and removed from `AppState`.

**Verification:**
* Unit test in `src/state.rs`: build a tab with one pane, dispatch `pane.close`, assert the tab is gone and the PTY is cleaned.
* Unit test: build a tab with two panes, dispatch `pane.close`, assert one pane remains and ratios are normalized.
* `cargo test state::` passes.

---

### Checkpoint 1 (after Phase A)

Before starting F5, confirm:

* [ ] All 13 actions registered and firing through the registry.
* [ ] `cargo test`, `cargo clippy`, `cargo fmt --check` green.
* [ ] User has manually verified `Ctrl+Shift+V`, `Ctrl+Shift+H`, `Ctrl+Shift+W` all work and that the existing `Ctrl+D`, `Ctrl+Shift+D`, `Ctrl+W` still work.
* [ ] Commit per feature (A1+A2 = "feat: keybind registry for split actions"; A3 = "feat: unsplit closes last-pane tab"). Two PRs.

---

### Phase B: Editable keybinds (F5)

**Goal:** user can remap every action from Settings > Keybinds, with conflict detection, reset, and JSON persistence. Unblocks further iteration without code rebuilds.

---

#### Task B1: JSON persistence layer

**Files**: `src/keybinds/loader.rs` (new).

**Acceptance:**
* `load_user_keybinds(path: &Path) -> Result<HashMap<KeybindAction, KeyCombo>, Error>`: reads JSON; missing file returns empty map (not error); malformed file logs warning and returns empty.
* `save_user_keybinds(path: &Path, map: &HashMap<KeybindAction, KeyCombo>) -> Result<()>`: writes JSON atomically (write to `.tmp`, rename).
* Path resolver: `keybinds_config_path() -> PathBuf` returns `<config_dir>/godly-terminal/keybindings.json`. Use `dirs::config_dir()` or whatever the app already uses for config (verify during implementation).

**Verification:**
* Unit tests cover: load-missing-returns-empty, load-malformed-returns-empty-with-log, round-trip write-then-read equals original, atomic write leaves no `.tmp` file on success.
* `cargo test keybinds::loader` passes.

**Open question to resolve during implementation:** confirm where the app currently resolves its config dir and reuse that; don't add a second config path.

---

#### Task B2: Keybind state in `AppState` and dispatch arms

**Files**: `src/state.rs`, `src/keybinds/recorder.rs` (new).

**Acceptance:**
* `AppState` gains `keybinds: HashMap<KeybindAction, KeyCombo>` populated on startup from `load_user_keybinds` + defaults for missing entries.
* New dispatch arms:
  * `keybind.set:<action_id>:<combo>` -> validates combo, checks conflicts; if conflict, sets `AppState.keybind_error` and does nothing; else updates map, calls `save_user_keybinds`, and rebinds the registry (`unregister` old + `register` new).
  * `keybind.reset:<action_id>` -> replaces with default; persists.
  * `keybind.reset_all` -> replaces all with defaults; persists.
* `RecordingState` struct in `recorder.rs` captures "which action is being recorded" + "pending combo"; lives in `AppState.keybind_recording: Option<RecordingState>`.

**Verification:**
* Unit tests:
  * `keybind.set` with non-conflicting combo updates state and persists.
  * `keybind.set` with conflicting combo leaves state unchanged and sets `keybind_error`.
  * `keybind.reset` restores default.
  * `keybind.reset_all` restores all 13 actions to defaults.
* Integration test: call `keybind.set`, restart `AppState` by reloading from the on-disk JSON, assert the new combo persists.

---

#### Task B3: Editable UI in Settings > Keybinds

**Files**: `src/ui/settings.rs`.

**Acceptance:**
* Replace the static `keybind_row()` list with a dynamic builder iterating `AppState.keybinds`.
* Each row: `Label | Combo button | Reset button | Conflict indicator`.
* Clicking the combo button sets `keybind_recording = Some(action)` and the cell shows "Press keys...".
* While recording, the app intercepts the next key combo (not through normal dispatch); captures it, calls `keybind.set`. `Escape` cancels.
* Conflict indicator: when `AppState.keybind_error` targets this row, show red border + tooltip with the conflicting action name.
* `Reset all` button in the section header fires `keybind.reset_all`.

**Verification:**
* Unit test: `build_keybinds_section()` renders 13 rows.
* Integration test via framework test harness: click a combo, send a key event, verify the new combo is persisted and visible in the row.
* `cargo run` and visually verify: change `Ctrl+T` to `Ctrl+Shift+T`, confirm new terminal hotkey works, close and reopen app, confirm persistence. User does visual verification per memory.

---

### Checkpoint 2 (after Phase B)

* [ ] Settings > Keybinds shows editable rows for all 13 actions.
* [ ] Conflict detection blocks invalid sets and surfaces which action conflicts.
* [ ] JSON persists across restart.
* [ ] `cargo test`, `cargo clippy`, `cargo fmt --check` green.
* [ ] User has verified remap + restart + conflict flow manually.
* [ ] PR landed ("feat: editable keybinds with conflict detection and persistence").

---

### Phase C: Pane extract to tab (F4)

**Goal:** pane header is a drag handle; dragging to tab bar creates a new tab.

---

#### Task C1: Pane header visibility + drag grip

**Files**: `src/ui/terminal_grid.rs`, `assets/styles.css`.

**Acceptance:**
* `build_pane_header()` is rendered only when the current tab has more than one pane. When `panes.len() == 1`, the header is omitted (not just hidden via CSS) so the terminal gets the full pane area.
* Header shows a grip icon (6 dots or equivalent) on the left, the terminal title in the center, and existing metadata on the right.
* Header has `cursor: grab` and `cursor: grabbing` while dragging.
* Clicking (not dragging) the header focuses the pane.

**Verification:**
* Unit test on the builder: single-pane tab has no header; multi-pane tab has header.
* Visual verification: split a tab, see headers appear on both panes; close one, remaining header disappears.

---

#### Task C2: Drag state machine and dispatch

**Files**: `src/drag/mod.rs` (new), `src/state.rs`.

**Acceptance:**
* `DragState` enum: `Idle`, `DraggingPane { source_pane: PaneId, cursor: (f32, f32) }`.
* `AppState.drag: DragState` field.
* New dispatch arms:
  * `drag.start_pane:<pane_id>` -> set `DragState::DraggingPane`.
  * `drag.update:<x>:<y>` -> update cursor; recompute hover target.
  * `drag.end:<x>:<y>` -> resolve drop via hit-test; route to `pane.extract_to_tab` or cancel.
  * `pane.extract_to_tab:<pane_id>:<index>` -> move the pane's terminal+PTY into a new `TerminalTab` inserted at `index`; remove it from the source tab; reflow ratios on the source tab; focus the new tab.

**Verification:**
* Unit tests for `pane.extract_to_tab`:
  * Extracting from a 2-pane tab leaves one pane in source (ratios normalized) and creates new tab at specified index.
  * Extracting the only pane from its tab closes the source tab (matches F3 semantics).
  * PTY and terminal state survive the move (no respawn).
* Unit test for drag state transitions.

**Open question to resolve:** confirm the PTY handoff works by moving the `Arc<Mutex<Terminal>>` rather than killing and respawning. If the PTY is keyed by tab id anywhere, that mapping must update. Check during implementation; if this requires nontrivial refactoring, surface before proceeding.

---

#### Task C3: Pane header drag handler and tab-bar drop target

**Files**: `src/ui/terminal_grid.rs` (attach `on_drag` to header), `src/ui/tabbar.rs` (drop zones + insertion placeholder).

**Acceptance:**
* Dragging the pane header more than 4px dispatches `drag.start_pane:<id>`, then `drag.update:<x>:<y>` per frame.
* When the cursor is within the tab bar's bounding rect, the tab bar shows a vertical placeholder between tabs at the computed insertion index.
* `DragPhase::End` dispatches `drag.end:<x>:<y>`; if over the tab bar, dispatches `pane.extract_to_tab`. Otherwise cancels (for now; F1 extends this to drop-on-other-pane).

**Verification:**
* Integration test simulating a drag from a pane header into the tab bar; asserts a new tab exists at the expected index and the source tab is reflowed.
* `cargo run` and visually verify: split a pane, drag its header up to the tab bar, see placeholder, drop, see a new tab. User does visual verification per memory.

---

### Checkpoint 3 (after Phase C)

* [ ] Pane header is a working drag source.
* [ ] Dropping on the tab bar creates a new tab with the pane's terminal intact.
* [ ] PTY is not respawned (verified by shell state: history, cwd, running process survive).
* [ ] Source tab closes if it had only one pane.
* [ ] `cargo test`, `cargo clippy`, `cargo fmt --check` green.
* [ ] User has verified the drag visually.
* [ ] PR landed ("feat: drag pane header to tab bar to extract as tab").

---

### Phase D: Tab drag with 5-zone drop (F1)

**Goal:** drag any tab, get a 5-zone drop preview in every pane, drop to split or reorder.

---

#### Task D1: Drop-zone hit-testing

**Files**: `src/drag/drop_zones.rs` (new).

**Acceptance:**
* `pub enum DropZone { Top, Right, Bottom, Left, Center }`.
* `pub fn hit_test(pane_rect: Rect, cursor: (f32, f32)) -> Option<DropZone>`: returns `None` if cursor is outside; otherwise returns the zone. Edge zones occupy the outer 25% of each side; center occupies the inner 50%.
* Ambiguity on corners: edge zones take priority over center; when two edges overlap (corner), the closer edge wins.

**Verification:**
* Unit tests covering: every zone hit, outside returns None, corner disambiguation, exact center, exact edge boundary.
* Table-driven tests with at least 20 cases.

---

#### Task D2: Drop-zone overlay rendering

**Files**: `src/drag/overlay.rs` (new), `src/ui/terminal_grid.rs` (conditional overlay when `DragState::DraggingTab`), `assets/styles.css`.

**Acceptance:**
* When `DragState::DraggingTab`, render a translucent overlay over every pane showing 5 zones (edge trapezoids + center rectangle).
* The zone currently under the cursor is filled with an accent color; others are subtly outlined.
* Overlay uses `pointer-events: none` except for the hit-test logic (drag events continue to fire on the underlying pane).

**Verification:**
* Visual verification after integration in D4. Unit tests cover the layout math if not trivial.

---

#### Task D3: Tab drag source and `tab.reorder` / `pane.drop_split` dispatch

**Files**: `src/ui/tabbar.rs`, `src/state.rs`, `src/drag/mod.rs` extended.

**Acceptance:**
* Each tab element has `on_drag`. Dragging more than 4px dispatches `drag.start_tab:<tab_id>`; extends `DragState` with `DraggingTab { source_tab: TabId, cursor: (f32, f32) }`.
* `DragPhase::Update` dispatches `drag.update:<x>:<y>`.
* `DragPhase::End` dispatches `drag.end:<x>:<y>`:
  * If cursor is within another tab's bounds -> `tab.reorder:<source>:<index>` (moves tab adjacent).
  * If cursor is within a pane and a zone is hit:
    * Edge -> `pane.drop_split:<target_pane>:<edge>` (moves source tab's active terminal into the target tab at the given edge).
    * Center -> for v1, reorder as an adjacent tab (true stacking deferred per spec).
  * Otherwise cancel.
* `pane.drop_split` dispatch: insert the source terminal as a new pane on the specified edge of the target pane, update `row_ratios` / `col_ratios`, remove from source tab (closing source tab if empty).

**Verification:**
* Unit tests for `pane.drop_split` on each of the four edges.
* Unit test for `tab.reorder`.
* Integration test simulating a full tab drag into another pane's edge zone; assert layout.

---

#### Task D4: Wire overlay to active drag and final visual QA

**Files**: `src/ui/terminal_grid.rs` (mount overlay), `src/ui/tabbar.rs` (source-tab ghost placeholder during drag).

**Acceptance:**
* While `DragState::DraggingTab`, every pane shows the 5-zone overlay and highlights the hovered zone.
* The source tab shows a faded "ghost" placeholder; the dragged tab label follows the cursor.
* Drop resolves per Task D3 semantics.

**Verification:**
* `cargo run` and visually verify the full flow:
  * Drag tab B onto tab A's right edge -> B's terminal becomes a right split of A, B tab is gone.
  * Drag tab B onto tab A's center -> B moves adjacent to A (no stacking in v1).
  * Drag tab B outside the window -> cancel.
* User does visual verification per memory.

---

### Checkpoint 4 (after Phase D)

* [ ] Tab drag with 5-zone preview works across all edges and center.
* [ ] All original keybinds still work.
* [ ] `cargo test`, `cargo clippy`, `cargo fmt --check` green.
* [ ] `cargo llvm-cov` shows no decrease vs. baseline.
* [ ] User has verified the full drag-drop-split flow.
* [ ] PR landed ("feat: tab drag with 5-zone drop into pane edges or center").

---

## Risk register

* **PTY handoff (F4/F1)**: if the PTY is stored keyed by tab id or pane id in a way that doesn't survive a move, we need to refactor the storage to key by a stable terminal id. Surface this in Task C2 if discovered; do not ship a workaround that respawns the shell.
* **Drag event ordering**: the framework's `DragEvent` is per-element. A tab's drag may stop firing when the cursor leaves the tab bar. Verify early; if true, attach a window-level pointer-move subscription for the duration of the drag (existing pattern in `terminal_grid.rs` resizers).
* **Overlay z-index**: the 5-zone overlay must render above the terminal canvas but not intercept clicks. Keep `pointer-events: none` and hit-test against `DragEvent.x/y` instead.
* **Keybind live rebind**: `ShortcutRegistry` supports `unregister` + `register`, but any cached resolver outside the app must also be refreshed. Check `bridge.rs` for cached bindings during Task B2.

## Out of scope (reaffirmed)

* True tab stacking (multiple terminals in one tab slot).
* Multi-window drag.
* Keybind profiles / presets.
* File-watcher hot-reload of `keybindings.json`.
* Changelog fragments (no `changelog/` dir in repo).
