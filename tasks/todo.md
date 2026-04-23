# TODO: Split Panel Functionality

Implementation checklist derived from `tasks/plan.md`. Check items off as they land.

## Phase A: Hotkey plumbing (F2 + F3)

### A1: `KeybindAction` enum and default-combo map [DONE]
- [x] Create `src/keybinds/mod.rs` with `KeybindAction` enum (13 variants)
- [x] Add `dispatch_command`, `default_combo`, `label`, `id` methods
- [x] Unit tests: id round-trip, default_combo parses, dispatch_command matches state.rs
- [x] `cargo test keybinds::` green (10/10)

### A2: Register defaults and route dispatch through registry [DONE]
- [x] Locate `ShortcutResolver` construction; register all 13 defaults on startup
- [x] Wire `Ctrl+Shift+V` (SplitRight alias), `Ctrl+Shift+H` (SplitDown alias), `Ctrl+Shift+W` (Unsplit)
- [x] Refactor `user_shortcut_bindings` to derive from `KeybindAction::ALL`
- [x] Unit tests: aliases, Ctrl+W = tab.close.active, Ctrl+Shift+W = pane.close, system shortcuts intact
- [ ] Visual verify: all three new combos plus all existing combos still work (pending user)

### A3: Last-pane-closes-tab semantics in `pane.close` [DONE]
- [x] Verified existing behavior (mutate_close_pane:984-991); no code change needed
- [x] Added `dispatch_pane_close_on_last_pane_closes_tab` regression test (locks down Unsplit semantics via dispatch path)
- [x] Added `close_pane_absorbs_ratio_into_neighbor` (pins ratio math)
- [x] 559/559 tests green

### Checkpoint 1
- [ ] `cargo test`, `cargo clippy`, `cargo fmt --check` green
- [ ] User visual verification sign-off
- [ ] PR 1: "feat: keybind registry for split actions"
- [ ] PR 2: "feat: unsplit closes last-pane tab"

---

## Phase B: Editable keybinds (F5)

### B1: JSON persistence [DONE]
- [x] Create `src/keybinds/loader.rs`
- [x] `load_user_keybinds` with missing-file and malformed-file fallbacks + unknown-id/bad-combo skip
- [x] `save_user_keybinds` with atomic write (`.tmp` + rename)
- [x] Config dir matches existing app usage (`com.godly.terminal` namespace under `dirs::config_dir()`)
- [x] 10/10 unit tests green (round-trip, missing, malformed, atomic, overwrite, parent-dir, empty, namespace)

### B2: `AppState.keybinds` + dispatch arms [DONE]
- [x] Add `keybinds: KeybindsState` to `AppState` (sparse overrides, recording, error)
- [x] Load overrides on startup via `load_if_installed` in `seed_state`
- [x] Implement `keybind.set:<action>:<combo>` with conflict + invalid-combo errors
- [x] Implement `keybind.reset:<action>` and `keybind.reset_all`
- [x] Add `keybind.record:<action>` and `keybind.cancel_record` dispatch arms
- [x] 12 KeybindsState unit tests + 8 dispatch tests green (all 589/589 suite)
- [x] `user_shortcut_bindings()` now honors saved overrides on startup (restart-to-apply per user's option-1 choice)
- [x] Persistence path installed in `main.rs` alongside `persist::install`

### B3: Editable Settings > Keybinds UI [DONE]
- [x] Replace static `keybind_row` block with dynamic builder over `AppState.keybinds`
- [x] Row layout: Label | Combo button | Reset button | Conflict indicator + restart/error banners
- [x] Click combo -> enter recording mode; "Press keys... (Esc to cancel)"
- [x] Framework `on_raw_key` hook added; `main.rs` consumes keys while recording
- [x] Intercept next combo; Escape cancels; any other combo fires `keybind.set`
- [x] Conflict indicator: red border on cell + error banner at top
- [x] "Reset all" button dispatches `keybind.reset_all`; per-row reset when overridden
- [x] 6 new unit tests green (section children count, row content, recording state, override reset button, error banner)
- [x] CSS for banner, error banner, recording cell, conflict cell, per-row reset
- [ ] Visual verify: remap, restart, confirm persistence (pending user)

### Checkpoint 2
- [ ] `cargo test`, `cargo clippy`, `cargo fmt --check` green
- [ ] User visual sign-off on remap + restart + conflict flow
- [ ] PR 3: "feat: editable keybinds with conflict detection and persistence"

---

## Phase C: Pane extract to tab (F4)

### C1: Pane header visibility + grip
- [ ] `build_pane_header` renders only when `panes.len() > 1`
- [ ] Add grip icon on the left; title center; metadata right
- [ ] CSS: `cursor: grab` / `grabbing`
- [ ] Click (not drag) focuses the pane
- [ ] Unit test: single-pane tab omits header; multi-pane tab includes it
- [ ] Visual verify

### C2: Drag state + `pane.extract_to_tab` dispatch
- [ ] Create `src/drag/mod.rs` with `DragState` enum (`Idle`, `DraggingPane`)
- [ ] Add `AppState.drag`
- [ ] Implement `drag.start_pane`, `drag.update`, `drag.end` dispatch arms
- [ ] Implement `pane.extract_to_tab:<pane>:<index>`: move terminal+PTY to new tab, reflow source
- [ ] Confirm PTY handoff works without respawn (or surface the blocker)
- [ ] Unit tests: extract from 2-pane tab; extract only pane -> tab closes; PTY survives

### C3: Header `on_drag` + tab bar drop target
- [ ] Attach `on_drag` to the pane header element
- [ ] Drag > 4px -> dispatch start + update events
- [ ] Tab bar computes insertion index from cursor x; renders vertical placeholder
- [ ] `DragPhase::End` over tab bar -> `pane.extract_to_tab`
- [ ] Integration test: simulate full drag; assert new tab + reflow
- [ ] Visual verify

### Checkpoint 3
- [ ] PTY state survives extraction (history, cwd, running process intact)
- [ ] `cargo test`, `cargo clippy`, `cargo fmt --check` green
- [ ] User visual sign-off
- [ ] PR 4: "feat: drag pane header to tab bar to extract as tab"

---

## Phase D: Tab drag with 5-zone drop (F1)

### D1: Drop-zone hit-testing
- [ ] Create `src/drag/drop_zones.rs`
- [ ] `DropZone` enum + `hit_test(rect, cursor) -> Option<DropZone>`
- [ ] Edge zones = outer 25%; center = inner 50%; corner disambiguation
- [ ] 20+ table-driven unit tests

### D2: Overlay rendering
- [ ] Create `src/drag/overlay.rs`
- [ ] Render 5-zone overlay per pane when `DragState::DraggingTab`
- [ ] Highlight hovered zone; others outlined
- [ ] `pointer-events: none` on overlay; hit-test uses `DragEvent.x/y`
- [ ] CSS additions

### D3: Tab drag source + `pane.drop_split` + `tab.reorder`
- [ ] Attach `on_drag` to each tab
- [ ] Extend `DragState` with `DraggingTab { source_tab, cursor }`
- [ ] Implement `drag.start_tab`, update, end
- [ ] End resolution: edge zone -> `pane.drop_split`; center/tab bar -> `tab.reorder`
- [ ] Implement `pane.drop_split:<target>:<edge>` (left|right splits column, top|bottom splits row)
- [ ] Implement `tab.reorder:<source>:<index>`
- [ ] Source tab removed (or closed if empty) after successful edge drop
- [ ] Unit tests per edge + reorder

### D4: Wire overlay + ghost placeholder
- [ ] Mount overlay conditionally in `build_terminal_grid`
- [ ] Source tab shows faded ghost; dragged label follows cursor
- [ ] Drop outside window cancels
- [ ] Visual verify:
  - [ ] Drag B onto A's right edge -> B becomes right split of A
  - [ ] Drag B onto A's center -> B moves adjacent to A
  - [ ] Drag B outside window -> cancel

### Checkpoint 4
- [ ] All prior keybinds still work
- [ ] `cargo test`, `cargo clippy`, `cargo fmt --check` green
- [ ] `cargo llvm-cov` no regression
- [ ] User visual sign-off on full flow
- [ ] PR 5: "feat: tab drag with 5-zone drop into pane edges or center"

---

## Release

- [ ] All 5 PRs merged into `feat/rust-terminal-manager`
- [ ] Final `cargo run` smoke test: every feature works end-to-end
- [ ] SPEC.md marked as implemented; open questions in §10 resolved or closed
