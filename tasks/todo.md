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

### B1: JSON persistence
- [ ] Create `src/keybinds/loader.rs`
- [ ] `load_user_keybinds` with missing-file and malformed-file fallbacks
- [ ] `save_user_keybinds` with atomic write (`.tmp` + rename)
- [ ] Confirm config dir resolver matches existing app usage
- [ ] Unit tests: round-trip, missing, malformed, atomic

### B2: `AppState.keybinds` + dispatch arms
- [ ] Add `keybinds: HashMap<KeybindAction, KeyCombo>` to `AppState`
- [ ] Load on startup, defaults fill gaps
- [ ] Implement `keybind.set:<action>:<combo>` with conflict check
- [ ] Implement `keybind.reset:<action>` and `keybind.reset_all`
- [ ] Add `keybind_recording: Option<RecordingState>` + `keybind_error: Option<KeybindError>`
- [ ] Unit tests for each dispatch arm
- [ ] Persistence integration test: set, reload state from disk, combo persists

### B3: Editable Settings > Keybinds UI
- [ ] Replace static `keybind_row` block with dynamic builder over `AppState.keybinds`
- [ ] Row layout: Label | Combo button | Reset button | Conflict indicator
- [ ] Click combo -> enter recording mode; "Press keys..."
- [ ] Intercept next combo; Escape cancels; any other combo fires `keybind.set`
- [ ] Conflict indicator: red border + tooltip with conflicting action name
- [ ] "Reset all" button dispatches `keybind.reset_all`
- [ ] Unit test: 13 rows rendered
- [ ] Integration test: click + key event = persisted change visible in UI
- [ ] Visual verify: remap, restart, confirm persistence

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
