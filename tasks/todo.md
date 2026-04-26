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

- [ ] **Task 6: Per workspace `shell` field**
  - [ ] Add `shell: ShellSpec` to `state::Workspace` (initialized in `new_workspace`).
  - [ ] Add `shell: ShellSpec` to `persist::PersistedWorkspace` with `#[serde(default)]`.
  - [ ] Update `from_state` and the workspace restoration loop in `main.rs`.
  - [ ] Spawn sites resolve via `shell::resolve(Some(&active_ws.shell), Some(&state.default_shell))`.
  - [ ] Test: workspace override beats app default in `shell::resolve`.
  - [ ] Test: state level dispatch with workspace 0 override + workspace 1 no override hits the right shell on each.
  - [ ] Persist round trip test covers both fields.
  - [ ] `cargo test` green.

### Checkpoint
- [ ] `cargo test` green.
- [ ] Manual: give two workspaces different `shell.program` values via `workspaces.json`, restart, open a tab in each, confirm each opens the right shell.

## Phase 5: Discovery and dispatch

- [ ] **Task 7: `discover_installed` shell scan**
  - [ ] Implement `shell::discover_installed() -> Vec<PathBuf>`.
  - [ ] PATH walk for known stems: `pwsh`, `powershell`, `cmd`, `bash`, `zsh`, `fish`, `nu`, `wsl`.
  - [ ] Windows: probe `C:\Program Files\Git\bin\bash.exe` and `C:\Windows\System32\wsl.exe`.
  - [ ] Deduplicate by canonical path; cap at 16 entries.
  - [ ] Test: returns at least one entry on the test host.
  - [ ] Test: stable order across calls.
  - [ ] Test: empty / unset PATH does not panic.
  - [ ] `cargo test --lib shell::discover_installed_` green on Windows and Unix.

- [ ] **Task 8: Dispatch handlers and persist trigger**
  - [ ] Add handlers for `shell.set_default:<json>`, `shell.set_workspace:<idx>:<json>`, `shell.clear_default`, `shell.clear_workspace:<idx>`.
  - [ ] Each parses, mutates, and persists.
  - [ ] Test: each command updates the right state field and triggers persist.
  - [ ] Test: malformed JSON returns false, no state change, no panic.
  - [ ] Test: out of range workspace index returns false.
  - [ ] `cargo test --lib state::dispatch_shell` green.

### Checkpoint
- [ ] `cargo test`, `cargo clippy`, `cargo fmt --check` clean.

## Phase 6: UI surfaces

- [ ] **Task 9: Settings → Shell tab full editor; remove General placeholder**
  - [ ] App default editor: dropdown of `discover_installed` results + "Custom path..." entry that reveals an editable program field.
  - [ ] Always visible args text field bound to `state.default_shell.args`.
  - [ ] Per workspace overrides subsection: one row per workspace, same dropdown / custom / args layout.
  - [ ] Each interaction dispatches the matching `shell.*` command.
  - [ ] Remove the "Default shell" row from `build_general_section` (`src/ui/settings.rs:108`).
  - [ ] Update or add UI snapshot tests for new structural classes.
  - [ ] `cargo test --lib ui::settings::` green.
  - [ ] **Manual smoke test (per CLAUDE.md):**
    - [ ] `cargo run`, open Settings → Shell.
    - [ ] Pick a non default discovered shell, confirm new pane opens it.
    - [ ] Pick "Custom path..." with a non discovered path, confirm a new pane uses it.
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
