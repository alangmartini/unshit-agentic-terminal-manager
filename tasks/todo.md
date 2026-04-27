# TODO: Configurable default shell

Implementation checklist derived from `tasks/plan.md`. Check items off as they land. Each task ends with `cargo test && cargo clippy --all-targets --all-features && cargo fmt --check` clean.

## Phase 1: Foundation

- [x] **Task 1: `ShellSpec` type + `resolve()`** [DONE]
  - [x] Create `src/shell.rs` with `ShellSpec { program, args }`, `is_empty()`, and `resolve(workspace, app)`.
  - [x] Add `pub mod shell;` to `src/main.rs`.
  - [x] Unit tests: `is_empty`, `resolve` (workspace wins, fallback, both empty), serde round trip with empty args defaulted in.
  - [x] `cargo test --bin terminal-manager shell::` green (10/10).

### Checkpoint
- [x] `cargo test`, `cargo clippy`, `cargo fmt --check` clean (terminal-manager package). No behavior change.

## Phase 2: Daemon honors `shell_args`

- [x] **Task 2: Additive `shell_args` field on `SpawnSession`** [DONE]
  - [x] Add `shell_args: Vec<String>` (with `#[serde(default, skip_serializing_if = "Vec::is_empty")]`) to `Request::SpawnSession`.
  - [x] Update `Client::spawn_session` signature.
  - [x] Update `Session::spawn` to forward args via new `pty::build_spawn_args` helper.
  - [x] Test: old wire payload (no `shell_args` key) deserializes with empty vector.
  - [x] Test: round trip with `shell_args = ["--login", "-i"]`.
  - [x] Test: empty `shell_args` is omitted from the wire (old daemon compat).
  - [x] Test: IPC integration test spawns `cmd.exe /C echo MARKER` (Windows) / `/bin/sh -c echo MARKER` (Unix), asserts MARKER in output stream.
  - [x] Test: `build_spawn_args` appends PowerShell `-NoExit -Command "Set-Location ..."` AFTER user args.
  - [x] Updated 12 test call sites + handler + registry + UI shim (`src/pty.rs`).
  - [x] `cargo test -p unshit-ptyd` green (141 unit + all integration tests).

- [x] **Task 3: `DaemonPty` shim carries `ShellSpec`** [DONE]
  - [x] `DaemonPty::spawn_in` and `attach_or_spawn` accept `Option<&ShellSpec>`.
  - [x] `Command::Spawn` enum variant carries `(shell, shell_args)`.
  - [x] Worker forwards them to `client.spawn_session`.
  - [x] Updated 4 call sites in `src/main.rs:458` and `src/state.rs:868`, `:1073`, `:1132` to pass `None` (Task 4 wires real values).
  - [x] Updated 1 call site in `src/bridge.rs:233` to pass `None`.
  - [x] Private helper `shell_spec_to_wire` normalizes `None` and empty `ShellSpec` to `(None, vec![])`.
  - [x] Tests: 3 unit (`shell_spec_to_wire_*`) + 1 integration (`spawn_in_with_shell_spec_routes_program_to_daemon`).
  - [x] `cargo test --bin terminal-manager pty::` green (19/19); full suite 823 pass.

### Checkpoint
- [x] `cargo test`, `cargo clippy`, `cargo fmt --check` clean.
- [ ] `cargo run` opens default shell exactly as before (no UI for shell selection yet).

## Phase 3: App wide default

- [x] **Task 4: `AppState.default_shell` + persistence + first time inference + initial spawn** [DONE]
  - [x] Added `default_shell: ShellSpec` to `AppState` (initialized in `seed_state` via `infer_default_shell(&discover_installed())`).
  - [x] Added `default_shell: ShellSpec` to `UiSnapshot`.
  - [x] Added `default_shell: ShellSpec` to `PersistedState` with `#[serde(default)]`.
  - [x] Wired load / save (`from_state`, restoration in `main.rs:391`).
  - [x] First time inference in `seed_state`: prefers `pwsh`, then `powershell`, else empty (daemon falls back).
  - [x] `discover_installed()` minimal PATH probe for `pwsh` / `powershell` (full impl in Task 7).
  - [x] Updated `main.rs:458` initial spawn to call `shell::resolve(None, Some(&guard.default_shell))`.
  - [x] Test: old `workspaces.json` without `default_shell` loads with `ShellSpec::default()` (`deserializes_with_default_shell_when_field_is_missing`).
  - [x] Test: round trip preserves a non default `default_shell` (`round_trip_preserves_non_default_default_shell`).
  - [x] Test: inference with `[pwsh, powershell]` picks `pwsh` (`infer_default_shell_prefers_pwsh_over_powershell`).
  - [x] `cargo test --bin terminal-manager`: 830 passed (up from 823, +7 new). clippy + fmt clean.

- [x] **Task 5: All other spawn sites resolve and pass spec** [DONE]
  - [x] Updated three sites in `src/state.rs` (`mutate_add_tab`, `mutate_split_right`, `mutate_split_down`) to use new `pane_spawn_shell(state)` helper.
  - [x] Updated `src/bridge.rs:233` resize-loop attach_or_spawn to use the same helper.
  - [x] Grep audit: only the `DaemonPty::spawn` shorthand still passes `None` literally and no production code calls it.
  - [x] Test: `add_tab_records_resolved_default_shell_on_pty_shim` (dispatch with `default_shell` set spawns the configured shell).
  - [x] Test: `add_tab_with_empty_default_shell_does_not_record_a_shell` (daemon falls back when default cleared).
  - [x] Test: `split_right_records_*` and `split_down_records_*` cover the split sites.
  - [x] Added `DaemonPty::spawn_shell` accessor (mirrors `spawn_cwd`) so unit tests can assert what was forwarded without standing up a daemon.
  - [x] `cargo test --bin terminal-manager`: 836 passed (up from 830, +6 new). clippy + fmt clean.

### Checkpoint
- [x] `cargo test` green.
- [ ] Manual: edit `workspaces.json` to set `"default_shell": { "program": "/bin/bash", "args": [] }`, restart, open a new pane, confirm `bash` runs (`echo $0`).

## Phase 4: Per workspace override

- [x] **Task 6: Per workspace `shell` field** [DONE]
  - [x] Added `shell: ShellSpec` to `state::Workspace` (initialized in `new_workspace`, `seed_state`, all test helpers).
  - [x] Added `shell: ShellSpec` to `persist::PersistedWorkspace` with `#[serde(default)]`.
  - [x] Updated `from_state` to copy `w.shell` and `main.rs` restoration loop to thread `entry.shell` back.
  - [x] `pane_spawn_shell` now consults the active workspace via `shell::resolve(Some(&active_ws.shell), Some(&state.default_shell))`.
  - [x] Test: `pane_spawn_shell_prefers_active_workspace_shell_over_app_default`.
  - [x] Test: `pane_spawn_shell_uses_correct_workspace_after_switch` (state-level dispatch with two workspaces).
  - [x] Test: `pane_spawn_shell_falls_back_to_app_default_when_workspace_shell_is_empty`.
  - [x] Persist round trip + missing-field tests cover `PersistedWorkspace.shell`.
  - [x] `cargo test --bin terminal-manager`: 841 passed (up from 836, +5 new). clippy + fmt clean.

### Checkpoint
- [x] `cargo test` green.
- [ ] Manual: give two workspaces different `shell.program` values via `workspaces.json`, restart, open a tab in each, confirm each opens the right shell.

## Phase 5: Discovery and dispatch

- [x] **Task 7: `discover_installed` shell scan** [DONE]
  - [x] Replaced the Task 4 stub with a full PATH walk over `STEMS = [pwsh, powershell, cmd, bash, zsh, fish, nu, wsl]`.
  - [x] Windows: probes `C:\Program Files\Git\bin\bash.exe` and `C:\Windows\System32\wsl.exe` via `fixed_well_known_paths`.
  - [x] Dedupes by `std::fs::canonicalize` so symlinked / aliased shells collapse to one entry.
  - [x] Caps at `MAX_DISCOVERED = 16`; `'walk` label breaks early instead of overshooting.
  - [x] Extracted `discover_from(path_dirs, fixed)` so unit tests can drive it without env access.
  - [x] Test: `discover_from_finds_known_shell_in_a_path_dir` (returns at least one entry).
  - [x] Test: `discover_installed_returns_stable_order_across_calls`.
  - [x] Test: `discover_from_returns_empty_when_no_dirs_and_no_fixed` + `discover_installed_does_not_panic` (empty / unset PATH).
  - [x] Tests: dedup-by-canonical, fixed-paths probe, missing-files-skipped, cap at MAX_DISCOVERED.
  - [x] `cargo test --bin terminal-manager shell::`: 24/24 green. clippy + fmt clean.

- [x] **Task 8: Dispatch handlers and persist trigger** [DONE]
  - [x] Added arms for `shell.set_default:<json>`, `shell.set_workspace:<idx>:<json>`, `shell.clear_default`, `shell.clear_workspace:<idx>` in the main `dispatch` match.
  - [x] Each helper parses, mutates, then calls `crate::persist::save_workspaces(state)`.
  - [x] Test: `dispatch_shell_set_default_updates_state` and `dispatch_shell_set_workspace_updates_correct_workspace` cover the happy path.
  - [x] Test: malformed JSON / missing payload / non-numeric index / out-of-range index all return false without panic (`with_malformed_*`, `with_out_of_range_index_*`).
  - [x] Test: `dispatch_shell_clear_default_when_already_empty_still_returns_true` covers idempotent clear.
  - [x] `cargo test --bin terminal-manager state::tests::dispatch_shell`: 12/12 green.

### Checkpoint
- [x] `cargo test`, `cargo clippy`, `cargo fmt --check` clean.

## Phase 6: UI surfaces

- [x] **Task 9: Settings → Shell tab full editor; remove General placeholder** [DONE]
  - [x] App default editor: chip group of `discover_installed` results + always visible custom path text input (Enter to apply, framework's `Tag::Input` doesn't seed initial value so placeholder shows current).
  - [x] Always visible args text input under each scope; submit splits on whitespace and dispatches matching `shell.set_*`.
  - [x] Per workspace overrides subsection: one `shell-scope-block` per workspace, with extra "Use default" chip that dispatches `shell.clear_workspace:<idx>`.
  - [x] Each chip / submit dispatches via the new `ShellScope::{AppDefault, Workspace(idx)}` enum so no command strings are duplicated.
  - [x] Removed the "Default shell" placeholder row from `build_general_section` and its now-orphan `select_display` helper + test.
  - [x] Updated `general_section_has_title_and_five_rows` (was six) and added 7 new structural tests (`shell_section_starts_with_app_default_block`, `shell_section_includes_shell_picker_under_app_default_block`, `shell_picker_marks_active_chip_when_program_matches`, `shell_picker_for_workspace_includes_use_default_chip`, `shell_picker_for_app_default_omits_use_default_chip`, `shell_section_has_one_workspace_override_block_per_workspace`, `general_section_no_longer_has_default_shell_row`).
  - [x] `cargo test --bin terminal-manager ui::settings`: 80 passed. clippy + fmt clean. Full suite 867.
  - [ ] **Manual smoke test (per CLAUDE.md, deferred to user):**
    - [ ] `cargo run`, open Settings → Shell.
    - [ ] Pick a non default discovered shell, confirm new pane opens it.
    - [ ] Type a non discovered path into the custom input, confirm a new pane uses it.
    - [ ] Set a workspace override, confirm it wins inside that workspace.
    - [ ] Restart, confirm picks survived.

- [ ] **Task 10: Workspace context menu shell submenu**
  - [ ] Locate the existing workspace context menu builder (likely `src/ui/sidebar.rs` or `src/ui/ctx_menu.rs`).
  - [ ] Add a "Shell" submenu listing `discover_installed` results plus "Use app default" when an override is set.
  - [ ] Picking an entry dispatches `shell.set_workspace:<idx>:<json>`; "Use app default" dispatches `shell.clear_workspace:<idx>`.
  - [ ] Mark the currently selected entry as active.
  - [ ] Unit test: menu builder structural test (label list + active marker + "Use app default" visible only when override set).
  - [ ] Manual: right click each workspace, confirm the submenu works end to end.

### Checkpoint
- [ ] Pause for human review before continuing to Phase 7.

## Phase 7: Cleanup

- [ ] **Task 11: Remove bench `SHELL=cmd.exe` override**
  - [ ] Delete `std::env::set_var("SHELL", "cmd.exe")` at `src/main.rs:325`.
  - [ ] Add regression test asserting the string is absent from `main.rs` source.
  - [ ] If bench mode needs a deterministic shell, route it through `default_shell` config instead of env var mutation.
  - [ ] `cargo test --lib bench_no_set_shell` green.
  - [ ] `cargo build --features bench` succeeds (manual sanity).

### Final checkpoint
- [ ] All success criteria from `SPEC.md` verified.
- [ ] `cargo test`, `cargo clippy --all-targets --all-features`, `cargo fmt --check` clean.
- [ ] Full manual smoke test of the user flow performed (both UI surfaces, per workspace and app default).
- [ ] PR ready for review.
