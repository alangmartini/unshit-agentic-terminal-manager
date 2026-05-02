# TODO: Quick Prompt overlay

Implementation checklist derived from `tasks/plan.md`. Check items off as they land. Each task ends with `cargo test && cargo clippy --all-targets --all-features && cargo fmt --check` clean.

## Phase 1: Foundation

- [ ] **Slice 0: Framework clipboard image API**
  - [ ] Add `ClipboardContent::Image { width: usize, height: usize, bytes: Vec<u8> }` to `crates/unshit-framework/crates/unshit-app/src/clipboard.rs`.
  - [ ] Add `ClipboardFormat::Image`.
  - [ ] Add `ClipboardContext::read_image() -> Result<Option<ClipboardContent>, ClipboardError>`.
  - [ ] Update `available_formats()` to include `Image` when present.
  - [ ] Test: `read_image_round_trips` (write synthetic `arboard::ImageData`, read back).
  - [ ] Test: `read_image_returns_none_when_text_only`.
  - [ ] Test: `available_formats_includes_image_after_write`.
  - [ ] `cargo test -p unshit-app clipboard::` green.
  - [ ] `cargo clippy --all-targets`, `cargo fmt --check` clean.

- [ ] **Slice 1: Empty overlay (open + close)**
  - [ ] Add `KeybindAction::QuickPromptOpen` to `src/keybinds/mod.rs` (variant, ALL, id, label, dispatch_command, default_combo_str).
  - [ ] Update `all_has_seventeen_variants` test to expect 18.
  - [ ] Add `QuickPromptOpen.dispatch_command()` assertion to `dispatch_commands_match_state_rs`.
  - [ ] Create `src/quick_prompt/mod.rs` declaring `state` and `ui` submodules.
  - [ ] Create `src/quick_prompt/state.rs` with `pub struct QuickPromptState {}` (placeholder; Slice 2 fills it).
  - [ ] Create `src/quick_prompt/ui.rs` with `build_quick_prompt_overlay(snap, shared)` returning the empty card.
  - [ ] Add `pub mod quick_prompt;` to `src/main.rs`.
  - [ ] Add `pub quick_prompt: Option<QuickPromptState>` to `AppState` (line ~462) and `UiSnapshot` (line ~630).
  - [ ] Mirror in `snapshot_state` helper.
  - [ ] Add dispatch arms `quick_prompt.open` and `quick_prompt.close` in `state.rs`.
  - [ ] Extend `modal.close` arm to clear `quick_prompt` alongside other modal fields.
  - [ ] Render hook in `main.rs`: when `snap.quick_prompt.is_some()`, append a modal overlay div with backdrop click dispatching `quick_prompt.close`.
  - [ ] Add `.quick-prompt-overlay` rules to `assets/styles.css` (copy from `.modal-overlay` shape; centered card).
  - [ ] Test: `state::dispatch::quick_prompt_open_sets_state`.
  - [ ] Test: `state::dispatch::quick_prompt_close_clears_state`.
  - [ ] Test: `state::dispatch::modal_close_clears_quick_prompt`.
  - [ ] Manual: `cargo run`; Ctrl+Shift+Q opens empty card; Esc closes; backdrop click closes; hotkey toggles.

### Checkpoint: Foundation
- [ ] `cargo test`, `cargo clippy`, `cargo fmt --check` clean (whole workspace).
- [ ] Manual verification of Slice 1 (no other behavior regressed).
- [ ] Human review before Phase 2.

## Phase 2: Core feature

- [ ] **Slice 2: Prompt input + agent toggle + persistence**
  - [ ] Expand `QuickPromptState` with `prompt: String`, `agent: Agent`, `error: Option<String>`.
  - [ ] Add `pub enum Agent { Claude, Codex }` to `src/quick_prompt/state.rs` with `serde` derives.
  - [ ] Add `QuickPromptStore` (mirror `persist::PersistedState`) with `install`, `load`, `save`, default config path `~/.config/com.godly.terminal/quick_prompt.json`.
  - [ ] Wire `QuickPromptStore::install` in `main.rs` startup.
  - [ ] Dispatch arm `quick_prompt.input <text>`: replaces buffer.
  - [ ] Dispatch arm `quick_prompt.toggle_agent`: flips agent, calls `QuickPromptStore::save`.
  - [ ] On `quick_prompt.open`: load agent from store before constructing the state.
  - [ ] UI: render input textarea with placeholder "What should the agent do?" plus Claude/Codex chips.
  - [ ] UI: input on_change dispatches `quick_prompt.input`.
  - [ ] UI: Tab key on the input dispatches `quick_prompt.toggle_agent` (preventDefault on Tab).
  - [ ] CSS: input + chip styling.
  - [ ] Test: `quick_prompt::state::store_round_trip`.
  - [ ] Test: `quick_prompt::state::store_missing_file_defaults_to_claude`.
  - [ ] Test: `quick_prompt::state::store_malformed_json_defaults_to_claude`.
  - [ ] Test: `state::dispatch::quick_prompt_toggle_agent`.
  - [ ] Test: `state::dispatch::quick_prompt_input_replaces_buffer`.
  - [ ] Manual: open overlay, type, Tab toggles, restart app, agent retained.

- [ ] **Slice 3: Submit happy path (Claude only, no images, no autocomplete)**
  - [ ] Create `src/quick_prompt/spawn.rs` with `prepare_target`, `claude_shell_spec`, `TargetDir`, `TargetKind`.
  - [ ] `prepare_target`: detect git repo via `git rev-parse --is-inside-work-tree`; on success run `git worktree add <appdata-path> HEAD`; on failure or non-repo create plain dir.
  - [ ] Helper to compute the appdata worktree path: `%APPDATA%\com.godly.terminal\worktrees\godly-qp-<8-hex>`.
  - [ ] `claude_shell_spec(prompt) -> ShellSpec { program: "claude".into(), args: vec![prompt.into()] }`.
  - [ ] Add `mutate_add_quick_prompt_tab(state, prompt, agent)` to `state.rs` (mirrors `mutate_add_tab` shape).
  - [ ] Tab name: `qp: <truncate(prompt, 30)>`.
  - [ ] Dispatch arm `quick_prompt.submit`:
    - [ ] Empty prompt: set `error`, return `true`.
    - [ ] Codex: temporarily set `error = "Codex coming soon"`, return `true`.
    - [ ] Claude: call `prepare_target` then `mutate_add_quick_prompt_tab`; on success `quick_prompt = None`; on failure set `error`.
  - [ ] UI: textarea on_submit (Ctrl+Enter binding) dispatches `quick_prompt.submit`.
  - [ ] UI: render error chip when `state.error.is_some()`.
  - [ ] CSS: error chip.
  - [ ] Test: `quick_prompt::spawn::prepare_target_creates_worktree_in_repo` (uses temp git repo).
  - [ ] Test: `quick_prompt::spawn::prepare_target_creates_plain_dir_when_not_in_repo`.
  - [ ] Test: `quick_prompt::spawn::claude_shell_spec_uses_prompt_as_arg`.
  - [ ] Test: `state::dispatch::quick_prompt_submit_empty_sets_error`.
  - [ ] Test: `state::dispatch::quick_prompt_submit_success_closes_overlay`.
  - [ ] Test: `state::dispatch::quick_prompt_submit_failure_keeps_overlay`.
  - [ ] Manual from a git repo: open, type, Ctrl+Enter, verify new tab in `%APPDATA%\com.godly.terminal\worktrees\godly-qp-*` running Claude.
  - [ ] Manual from a non git temp dir: same flow creates a plain dir and runs Claude.

### Checkpoint: Core feature
- [ ] All tests + clippy + fmt clean.
- [ ] End to end submit verified (worktree + plain dir paths).
- [ ] Human review before Phase 3.

## Phase 3: Polish

- [x] **Slice 4: Image paste** (commit 47d5bdb)
  - [x] `src/quick_prompt/images.rs` with `QuickPromptImage`, `capture_clipboard_image`, `cleanup_session`, `move_into_target`, `append_image_references`.
  - [x] Hash bytes with 64-bit FNV-1a (16-hex). `sha2` skipped to avoid the dep; FNV-1a is sufficient for in-session de-dup. Spec deviation logged.
  - [x] Persist temp images at `temp_dir().join("godly-qp").join(<8-hex session id>).join(<hash>.png)` plus `<hash>.thumb.png`.
  - [x] Generate 96px max-edge thumbnail via `image::imageops::thumbnail` (workspace `image` crate, png+jpeg features already on).
  - [x] Added `session_hex` and `images: Vec<QuickPromptImage>` to `QuickPromptState`; fresh session_hex per open.
  - [x] Dispatch arm `quick_prompt.image_paste`: reads clipboard, dedups by hash, surfaces friendly error chip on empty clipboard or arboard failure.
  - [x] Image removal handled by direct mutation from the chip remove button (no separate dispatch arm needed; chip lives inside the overlay).
  - [x] Cleanup on close: `modal.close`, `quick_prompt.open` toggle off, `quick_prompt.close` all run `cleanup_session`.
  - [x] On submit: `move_into_target` relocates files into `<target>/.quick-prompt/<hash>.png` (with cross-volume copy fallback); `append_image_references` adds the block before `claude_shell_spec`.
  - [x] UI: toolbar with "Attach image" button + thumbnail chip strip below the input. Spec deviation: framework does not expose Ctrl+V paste events, so paste is button-driven for now.
  - [x] CSS: `.quick-prompt-toolbar`, `.quick-prompt-image-strip`, `.quick-prompt-image-chip`, `.quick-prompt-image-thumb`, `.quick-prompt-image-remove`.
  - [x] Re-exported `ClipboardContent` / `ClipboardFormat` from `unshit-app::lib`.
  - [x] Tests: 13 in `quick_prompt::images`, 2 in `quick_prompt::state` for session_hex, 4 dispatch tests covering paste + cleanup paths. 1002 total green.

- [ ] **Slice 5: Autocomplete (Claude only)**
  - [ ] Create `src/quick_prompt/autocomplete.rs` with `Entry`, `EntryKind`, `Popup`, `load_claude_sources`, `filter`.
  - [ ] Cache `Mutex<Option<(Instant, Vec<Entry>)>>`; reuse if <5s old.
  - [ ] Trigger detection: dispatch arm `quick_prompt.input` checks the new char; opens `Popup` when `/` follows whitespace or at start.
  - [ ] Popup state arms: `quick_prompt.autocomplete_select_next`, `_prev`, `_confirm`, `_dismiss`.
  - [ ] Confirm inserts `/<entry.name>` at cursor and closes popup.
  - [ ] UI: render popup absolutely positioned below the input; selected row highlighted.
  - [ ] Up/Down/Enter/Tab/Esc handled via input keybinds while popup is open.
  - [ ] CSS: popup container, row, hover, selected.
  - [ ] Test: `quick_prompt::autocomplete::load_claude_sources_walks_skills_and_commands` (uses temp dirs).
  - [ ] Test: `quick_prompt::autocomplete::filter_case_insensitive_substring`.
  - [ ] Test: `quick_prompt::autocomplete::filter_empty_query_returns_all`.
  - [ ] Test: `quick_prompt::autocomplete::trigger_detection_after_whitespace`.
  - [ ] Test: `quick_prompt::autocomplete::trigger_detection_at_start`.
  - [ ] Test: `quick_prompt::autocomplete::missing_dir_yields_empty`.
  - [ ] Bench: create `benches/quick_prompt_filter.rs` over 200 synthetic entries (criterion).
  - [ ] Add bench entry to `Cargo.toml`.
  - [ ] `cargo bench --bench quick_prompt_filter` p99 <1ms (verify on dev machine; document baseline).
  - [ ] Manual: open overlay, type `/`, popup appears with real claude entries; arrow keys navigate; Tab inserts; Esc dismisses.

- [ ] **Slice 6: Codex parity**
  - [ ] Verify OQ1: run `codex exec "say hi"` in a terminal to confirm CLI shape. Update `codex_shell_spec` if it differs.
  - [ ] `quick_prompt::spawn::codex_shell_spec(prompt) -> ShellSpec`.
  - [ ] Update `quick_prompt.submit` arm to dispatch on `agent`; remove the "Codex coming soon" stub.
  - [ ] `quick_prompt::autocomplete::load_codex_command_sources` (`~/.codex/prompts/*.md`).
  - [ ] `quick_prompt::autocomplete::load_codex_skill_sources` (`~/.codex/skills/*/`, exclude `.system`).
  - [ ] Trigger detection: when `agent == Codex` and char is `

` after whitespace or at start, open skill popup.
  - [ ] Popup uses correct source list based on `(agent, trigger)` tuple.
  - [ ] Test: `quick_prompt::spawn::codex_shell_spec_uses_exec_subcommand`.
  - [ ] Test: `quick_prompt::autocomplete::load_codex_skill_sources_excludes_dot_system`.
  - [ ] Test: `quick_prompt::autocomplete::trigger_detection_codex_backtick`.
  - [ ] Manual from a git repo: Tab to Codex, submit, verify new tab runs `codex exec`.
  - [ ] Manual: backtick autocomplete works; `.system/` not present.

- [ ] **Slice 7: Polish + perf gates + daemon name**
  - [ ] Add `DaemonPty::spawn_in_named` accepting `name: Option<&str>`; `spawn_in` delegates with `None`.
  - [ ] `mutate_add_quick_prompt_tab` uses `spawn_in_named` so the daemon stores `qp: <prompt prefix>` as session name.
  - [ ] Test: `pty::spawn_in_named_forwards_name`.
  - [ ] Sweep `quick_prompt.input` dispatch path for `RequestRebuild` opportunities; ensure rapid typing does not cause thrashing.
  - [ ] Document the autocomplete bench in `CLAUDE.md` or `scripts/bench.ps1`.
  - [ ] Final visual pass: focus management on open, error chip color matches toast palette, thumbnail strip alignment.
  - [ ] Final manual pass over user stories U1 to U6.

### Checkpoint: Complete
- [ ] All acceptance criteria F1 to F8 met.
- [ ] All tests + clippy + fmt clean across workspace.
- [ ] Bench gate met.
- [ ] Manual U1 to U6 verification recorded.
- [ ] Ready for review.

## Open questions to resolve before merge

* **OQ1** Codex `exec` flag confirmed (Slice 6).
* **OQ2** Prompt draft persistence across opens? Spec says no; confirm with user during Slice 2 review.
* **OQ4** Treat missing `~/.claude` or `~/.codex` dirs as zero entries (no error chip).
