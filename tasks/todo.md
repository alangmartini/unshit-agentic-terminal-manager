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

### C1: Pane header visibility + grip [DONE]
- [x] `build_pane` omits `build_pane_header` entirely when single_pane
- [x] Added `icon_grip` (6-dot SVG) and wired into `pane-header-left`
- [x] CSS: `.pane-grip` with `cursor: grab` / `grabbing`, opacity fades on hover/active
- [x] Click-to-focus on the pane container still works (grip inherits)
- [x] Updated existing test `pane_header_left_has_grip_dot_title_subtitle` for new 4-child order
- [x] New tests `single_pane_tab_omits_pane_header` and `multi_pane_tab_renders_header_with_grip`
- [x] 588/588 tests green, clippy clean

### C2: Drag state + `pane.extract_to_tab` dispatch [DONE]
- [x] Create `src/drag/mod.rs` with `DragState` enum (`Idle`, `DraggingPane`)
- [x] Add `AppState.drag` (+ `UiSnapshot.drag`, default Idle)
- [x] Implement `drag.start_pane:<pane>:<x>:<y>`, `drag.update:<x>:<y>`, `drag.end` dispatch arms (7 new tests)
- [x] Implement `pane.extract_to_tab:<pane>:<index>`: PTY handle preserved, ratios reflowed, new tab activated (9 new tests)
- [x] PTY handoff verified via Arc::strong_count — no respawn
- [x] 607/607 tests green, clippy/fmt clean

### C3: Header `on_drag` + tab bar drop target [DONE]
- [x] `on_drag` attached to `.pane-grip` in `build_pane_header`; framework handles 4px threshold
- [x] `DragPhase::Start/Update/End` dispatch to `drag.start_pane` / `drag.update` / `drag.end`
- [x] Tab bar `on_resize` records absolute `tabbar_rect` (x = sidebar + resizer, y = titlebar height)
- [x] `pane_drag_insertion_index` renders vertical `.tab-drop-placeholder` at computed slot
- [x] `drag.end` dispatch extracts pane into new tab when cursor lies in `tabbar_rect`
- [x] Pure hit-test helpers (`Rect::contains`, `resolve_tabbar_drop`) with 8 table-driven tests
- [x] Grip handler end-to-end test: simulate Start/Update/End -> new tab appears, drag state clears
- [x] 627/627 tests green, clippy clean, fmt clean
- [ ] Visual verify: drag pane grip onto tab bar, placeholder appears, drop creates new tab (pending user)

### Checkpoint 3
- [ ] PTY state survives extraction (history, cwd, running process intact)
- [ ] `cargo test`, `cargo clippy`, `cargo fmt --check` green
- [ ] User visual sign-off
- [ ] PR 4: "feat: drag pane header to tab bar to extract as tab"

---

## Phase D: Tab drag with 5-zone drop (F1)

### D1: Drop-zone hit-testing [DONE]
- [x] Create `src/drag/drop_zones.rs`
- [x] `DropZone` enum + `hit_test(rect, cursor) -> Option<DropZone>`
- [x] Edge zones = outer 25%; center = inner 50%; corner disambiguation (closer edge wins, tie -> vertical)
- [x] 21 table-driven unit tests green; 659/659 suite, clippy/fmt clean

### D2: Overlay rendering [DONE]
- [x] Create `src/drag/overlay.rs`
- [x] Render 5-zone overlay per pane when `DragState::DraggingTab`
- [x] Highlight hovered zone; others outlined
- [x] `pointer-events: none` on overlay; hit-test driven by `DragState` cursor + `pane_rects`
- [x] CSS additions (`.drop-zone-overlay`, `.drop-zone`, `.drop-zone.hovered`)
- [x] `DragState::DraggingTab { source_tab, cursor_x, cursor_y }` variant added; `drag.update` now refreshes cursor for both pane and tab drags
- [x] `AppState.pane_rects` and `pane.rect:<pane>:<x>:<y>:<w>:<h>` dispatch arm (6 new tests); overlay module has 14 unit tests; 678/678 suite green, clippy/fmt clean

### D3: Tab drag source + `pane.drop_split` + `tab.reorder` [DONE]
- [x] `on_drag` on each tab button dispatches `drag.start_tab:<id>:<x>:<y>` / `drag.update` / `drag.end`
- [x] `DragState::DraggingTab { source_tab, cursor }` (added in D2)
- [x] `drag.start_tab` / `drag.update` / `drag.end` dispatch arms
- [x] `drag.end` (tab variant): tab-bar -> `mutate_tab_reorder`; edge zone -> `mutate_pane_drop_split`; center zone -> reorder next to active tab
- [x] `pane.drop_split:<target>:<edge>` dispatch arm (reads source from drag state; rejects center and no-drag states)
- [x] `tab.reorder:<source>:<index>` dispatch arm (preserves active tab by id)
- [x] Source tab removed after edge drop (single-pane-source restriction for safety)
- [x] Replaced stored `pane_rects` with pure `compute_pane_rects` + `grid_rect_from_state` used by both overlay and drag-end hit-test
- [x] 20 new tests (reorder, drop_split per edge, dispatch arms, drag.end variants); 697/697 suite green, clippy/fmt clean

### D4: Wire overlay + ghost placeholder [DONE]
- [x] `build_drop_zone_overlay` mounted at root in `main.rs` (same layer as drag ghost, Position::Fixed)
- [x] Source tab gets `.tab.dragging` class (CSS fades it to 0.4 opacity); ghost extended to show tab label + subtitle
- [x] Drop outside window falls through `dispatch_drag_end` tab branch with no hit and clears drag state
- [x] 4 new tests (dragging-class on source tab only, ghost renders for tab drag with correct label, ghost `None` when dragged tab id missing)
- [x] 701/701 suite green, clippy clean, fmt clean
- [ ] Visual verify (pending user):
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
