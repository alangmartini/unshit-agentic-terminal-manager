# Plan: Configurable default shell

Derived from `SPEC.md`. Each task is a vertical slice: type changes + tests + the smallest end to end behavior change that proves the slice. Tasks are sequential per the project's "merge agents one at a time" guidance.

## Architecture decisions

* **`ShellSpec { program: String, args: Vec<String> }`** is the single representation everywhere (state, persist, IPC). Empty `program` means "fall back to the next level"; both empty means "let the daemon's `default_shell()` decide" (today's behavior).
* **Resolution function** `shell::resolve(workspace, app) -> Option<ShellSpec>` is called at every spawn site. The resolved spec (if `Some`) is forwarded to the daemon as `(shell, shell_args)`.
* **Wire change is additive**: `Request::SpawnSession` gains `shell_args: Vec<String>` defaulted via `#[serde(default)]`. No protocol version bump (per the contract in `protocol/message.rs`).
* **Daemon owns PowerShell quirk**: the existing `is_powershell_shell` + `build_powershell_cwd_args` runs after the user's args, unchanged.
* **Persistence reuses `workspaces.json`**: app default at the top, per workspace overrides inside each `PersistedWorkspace`. New fields default to `ShellSpec::default()` so old configs upgrade silently.
* **Two UI surfaces**: Settings → Shell tab (full editor with dropdown + custom path + args) and the workspace context menu in the sidebar (quick switcher: pick a discovered shell, no args editor).
* **Picker shape**: dropdown of `discover_installed()` results plus a "Custom path..." entry that reveals a text input. Args field is always visible and editable. (Confirmed during planning.)
* **First time inference**: pre populate `default_shell.program` on fresh install with `pwsh.exe` > `powershell.exe` > today's `default_shell()` value. Resolution still treats an empty program as "fall back."
* **MCP wiring is out of scope** (see SPEC.md "Decisions resolved during planning"). The MCP tool names exist in the registered tool list but are not implemented today; not part of this work.

## Dependency graph

```
Task 1: ShellSpec type + resolve()
   |
   +-> Task 2: Daemon protocol additive change (shell_args)
   |       |
   |       +-> Task 3: UI shim DaemonPty carries ShellSpec
   |               |
   |               +-> Task 4: AppState.default_shell + persist + first time inference
   |                       |
   |                       +-> Task 5: All other spawn sites use resolve()
   |                               |
   |                               +-> Task 6: Per workspace override
   |                                       |
   |                                       +-> Task 7: discover_installed
   |                                       |       |
   |                                       |       +-> Task 9: Settings tab picker
   |                                       |       +-> Task 10: Workspace context menu
   |                                       |
   |                                       +-> Task 8: Dispatch handlers
   |                                               |
   |                                               +-> Task 9: Settings tab picker
   |                                               +-> Task 10: Workspace context menu
   |
   +-> Task 11: Remove bench SHELL override
```

Tasks 7 and 8 are independent (could parallelize). Tasks 9 and 10 both depend on Tasks 7 and 8 but are independent of each other (different files; could parallelize). Task 11 has no dependencies and can land any time.

## Task list

### Phase 1: Foundation

- [ ] **Task 1: `ShellSpec` type and `resolve()`**

  **Description:** Add `src/shell.rs` containing the `ShellSpec` struct, a `resolve(workspace, app) -> Option<ShellSpec>` function, and unit tests for both. No call sites yet. Declare `pub mod shell;` in `src/main.rs`.

  **Acceptance criteria:**
  * `ShellSpec` derives `Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize`.
  * `is_empty()` returns true when `program` is empty (regardless of args).
  * `resolve` returns workspace override when both are non empty, app default when workspace is empty, `None` when both are empty.
  * Round trips through `serde_json` with empty args defaulted in.

  **Verification:**
  * `cargo test --lib shell::` passes.
  * `cargo clippy --all-targets --all-features` passes with no warnings.

  **Files likely touched:** `src/shell.rs` (new), `src/main.rs` (module declaration).

  **Estimated scope:** S (1 new file, 1 line in main.rs).

### Checkpoint after Task 1
* `cargo test` green; foundation compiles; no behavior change.

### Phase 2: Daemon honors shell_args

- [ ] **Task 2: Additive `shell_args` field on `SpawnSession`**

  **Description:** Add `shell_args: Vec<String>` (with `#[serde(default)]`) to `Request::SpawnSession`. Thread it through `Client::spawn_session` and `Session::spawn` so it lands at `CommandBuilder::arg`. Existing PowerShell cwd args are appended after the user's args.

  **Acceptance criteria:**
  * Old wire payload (no `shell_args` key) deserializes with an empty vector.
  * New `spawn_session_round_trips_with_shell_args` test asserts JSON contains `"shell_args":["--login"]` when set; absent or empty is acceptable when unset.
  * `Session::spawn` forwards each arg to `CommandBuilder::arg` in order.
  * IPC integration test spawns with `shell = "/bin/sh"` (Unix) or `cmd.exe` (Windows) plus a single arg that prints a recognizable token, asserts the token appears in the output stream.
  * PowerShell with user args + cwd still appends `-NoExit -Command "Set-Location ..."` after the user's args (regression test).
  * Existing tests still pass: every `client.spawn_session(...)` and `Session::spawn(...)` call site updated to the new signature.

  **Verification:**
  * `cargo test -p unshit-ptyd` passes.
  * `cargo clippy -p unshit-ptyd` passes.

  **Files likely touched:**
  * `crates/unshit-ptyd/src/protocol/message.rs`
  * `crates/unshit-ptyd/src/client/mod.rs`
  * `crates/unshit-ptyd/src/session/mod.rs`
  * `crates/unshit-ptyd/tests/ipc_session_spawn.rs`

  **Estimated scope:** M (4 files).

- [ ] **Task 3: `DaemonPty` shim carries `ShellSpec`**

  **Description:** `DaemonPty::spawn_in` and `DaemonPty::attach_or_spawn` accept `Option<&ShellSpec>`. The shim's internal `Command::Spawn` enum variant stores `(shell, shell_args)` so the worker forwards them to `client.spawn_session`. Test helpers updated to pass `None`.

  **Acceptance criteria:**
  * `spawn_in` and `attach_or_spawn` signatures take `shell: Option<&ShellSpec>` (borrowed; the worker clones into the IPC payload).
  * The 4 call sites in `src/main.rs:457` and `src/state.rs:868`, `:1073`, `:1132` updated to pass `None` (Task 4 wires the real value).
  * The 1 call site in `src/bridge.rs:233` updated to pass `None`.
  * `pty.rs` unit and integration tests still pass.
  * New `pty.rs` unit test: `spawn_in` with a `ShellSpec` whose program is empty is treated as `None` at the wire (i.e. `client.spawn_session` is called with `shell: None, shell_args: vec![]`).

  **Verification:**
  * `cargo test --lib pty::` passes.
  * `cargo build` succeeds.

  **Files likely touched:** `src/pty.rs`, `src/main.rs`, `src/state.rs`, `src/bridge.rs` (mechanical updates).

  **Estimated scope:** M (4 files; mostly call site mechanics).

### Checkpoint after Phase 2
* `cargo test` green; daemon honors shell_args programmatically; UI still passes `None` so behavior is unchanged from today.
* Manual: `cargo run` opens a default shell exactly as before. (No shell selection yet.)

### Phase 3: App wide default

- [ ] **Task 4: `AppState.default_shell` + persistence + first time inference + initial spawn**

  **Description:** Add `default_shell: ShellSpec` to `AppState` and `UiSnapshot`. Add `default_shell: ShellSpec` to `PersistedState` (with `#[serde(default)]`). Wire load/save. On fresh install (no `workspaces.json`), `seed_state()` infers the program: prefer `pwsh.exe` if `discover_installed` finds it, else `powershell.exe`, else today's `default_shell()` value. The initial pane spawn in `main.rs:457` resolves via `shell::resolve(None, Some(&state.default_shell))` and passes the result.

  **Acceptance criteria:**
  * Loading an old `workspaces.json` (no `default_shell` key) yields `ShellSpec::default()`; the inferred default is only applied when no config file exists.
  * `PersistedState` round trips with a non default `default_shell`.
  * `seed_state()` initializes `default_shell` via the inference rule.
  * Inference rule unit test: with a stubbed `discover_installed` returning `[pwsh.exe, powershell.exe]`, the inferred program is `pwsh.exe`.
  * Integration test: programmatically setting `state.default_shell = ShellSpec { program: "/bin/sh", args: vec![] }` and routing through the initial spawn flow results in the daemon receiving `shell: Some("/bin/sh")`.
  * `UiSnapshot` exposes `default_shell` so the future settings UI can read it.

  **Verification:**
  * `cargo test --lib state::` and `cargo test --lib persist::` pass.
  * `cargo test` green overall.

  **Files likely touched:** `src/state.rs`, `src/persist.rs`, `src/main.rs`, `src/shell.rs` (test helper).

  **Estimated scope:** M (4 files).

- [ ] **Task 5: All other spawn sites resolve and pass spec**

  **Description:** Update the three remaining spawn sites in `src/state.rs:868`, `:1073`, `:1132` and the reattach site in `src/bridge.rs:233` to call `shell::resolve(...)` with the active workspace's eventual override (still `None` until Task 6) and the app default.

  **Acceptance criteria:**
  * Every `pty_manager.spawn_in` and `pty_manager.attach_or_spawn` call resolves the spec from state and passes it through.
  * Grep audit: no remaining `pty_manager.spawn_in(.*, None)` or `attach_or_spawn(.*, None)` literals (should only appear in tests).
  * State level test: dispatching the new pane command (`tab.new` or equivalent) with `state.default_shell.program = "X"` results in the shim's recorded `last_spawn_shell` matching `Some("X")`. (Use a test seam on `DaemonPty` or assert via the IPC integration test below.)
  * Integration test: with `default_shell` set, opening a new tab spawns the configured shell. With it cleared, the daemon falls back to today's behavior.

  **Verification:**
  * `cargo test` green.
  * `cargo clippy` clean.

  **Files likely touched:** `src/state.rs`, `src/bridge.rs`, possibly a small test seam in `src/pty.rs`.

  **Estimated scope:** M (3 files).

### Checkpoint after Phase 3
* `cargo test` green.
* Manual: edit `workspaces.json` by hand to set `"default_shell": { "program": "/bin/bash", "args": [] }`, restart, open a new pane, confirm `bash` is running (`echo $0`).

### Phase 4: Per workspace override

- [ ] **Task 6: Per workspace `shell` field + resolve takes both levels**

  **Description:** Add `shell: ShellSpec` to `state::Workspace` and `persist::PersistedWorkspace` (both with `#[serde(default)]`). `new_workspace` initializes to `ShellSpec::default()`. Spawn sites pass `Some(&active_workspace.shell)` plus `Some(&state.default_shell)` to `shell::resolve`.

  **Acceptance criteria:**
  * Workspace serde round trips include the new field; old configs load with the default.
  * `shell::resolve` continues to satisfy its existing tests; new test asserts a non empty workspace override beats the app default.
  * State level test: setting `workspaces[0].shell.program = "X"` and `state.default_shell.program = "Y"`, then dispatching the new pane command on workspace 0, results in the shim seeing `X`. Same flow on workspace 1 sees `Y`.

  **Verification:**
  * `cargo test` green.
  * Persist round trip test in `src/persist.rs` covers both fields.

  **Files likely touched:** `src/state.rs`, `src/persist.rs`, `src/shell.rs` (test only), `src/bridge.rs` (resolve call), `src/main.rs` (initial spawn resolve call).

  **Estimated scope:** M (5 files; mostly small).

### Checkpoint after Phase 4
* `cargo test` green.
* Manual: edit `workspaces.json` to give two workspaces different `shell.program` values, restart, open a tab in each, confirm each opens the right shell.

### Phase 5: Discovery and dispatch (parallel safe)

- [ ] **Task 7: `discover_installed` shell scan**

  **Description:** Implement `shell::discover_installed() -> Vec<PathBuf>`. Walk `PATH` for known stems (`pwsh`, `powershell`, `cmd`, `bash`, `zsh`, `fish`, `nu`, `wsl`). On Windows also probe `C:\Program Files\Git\bin\bash.exe` and `C:\Windows\System32\wsl.exe`. Deduplicate by canonical path; preserve a stable, deterministic order.

  **Acceptance criteria:**
  * Returns at least one entry on the test host (sanity assertion).
  * Order is stable across calls with the same PATH.
  * Does not panic when PATH is unset or empty.
  * Skips entries that are not regular files / not executable.
  * Cap on number of returned entries (e.g. 16) so a pathological PATH cannot blow up the picker.

  **Verification:**
  * `cargo test --lib shell::discover_installed_` passes on Windows and Unix.

  **Files likely touched:** `src/shell.rs`.

  **Estimated scope:** S (1 file).

- [ ] **Task 8: Dispatch handlers and persist trigger**

  **Description:** Add `dispatch` handlers for `shell.set_default:<json>`, `shell.set_workspace:<idx>:<json>`, `shell.clear_default`, `shell.clear_workspace:<idx>`. Each parses (using `serde_json` on the trailing `<json>` payload), mutates state, and triggers `persist::save_workspaces`.

  **Acceptance criteria:**
  * Each command updates the corresponding state field.
  * Each command triggers a persist save (verified with the existing `install` test seam).
  * Malformed JSON returns false (no panic, no state change).
  * Out of range workspace index returns false.
  * Unit tests cover all four commands plus the malformed and out of range cases.

  **Verification:**
  * `cargo test --lib state::dispatch_shell` passes.

  **Files likely touched:** `src/state.rs`.

  **Estimated scope:** S (1 file).

### Checkpoint after Phase 5
* `cargo test` green.
* Programmatically dispatching the new commands now updates state and persists correctly. UI still doesn't expose them (Tasks 9 and 10).

### Phase 6: UI surfaces (parallel safe)

- [ ] **Task 9: Settings → Shell tab full editor; remove General placeholder**

  **Description:** In `build_shell_section` (`src/ui/settings.rs:199`), build the App default editor: a dropdown listing `discover_installed()` results plus a "Custom path..." entry that reveals an editable program text field; an always visible args text field bound to the same state. Below it, the "Per workspace overrides" subsection lists each workspace with the same dropdown + custom + args layout. Each interaction dispatches the matching `shell.*` command from Task 8. Remove the placeholder "Default shell" row from `build_general_section` (`src/ui/settings.rs:108`).

  **Note:** The settings modal is now `Display::Grid` (168px nav | body) per cbca91b; the shell section content lives inside the body grid cell.

  **Acceptance criteria:**
  * Selecting a discovered entry from the dropdown sets `state.default_shell.program` and persists.
  * Selecting "Custom path..." reveals a text field; typing into it sets `state.default_shell.program` and persists on commit (Enter or blur).
  * Editing the args text field sets `state.default_shell.args` and persists on commit.
  * Per workspace rows reflect each workspace's `shell` and dispatch the per workspace variants.
  * The "Default shell" row no longer appears in the General tab.
  * Existing `build_settings_modal` snapshot tests still pass; new tests cover the section's structural classes.

  **Verification:**
  * `cargo test --lib ui::settings::` passes.
  * **Manual smoke test (required per CLAUDE.md):**
    1. `cargo run`.
    2. Open Settings → Shell.
    3. Pick a non default shell (e.g. `pwsh`) from the dropdown.
    4. Open a new pane, run `$Host.Name` (PowerShell) / `echo $0` (bash) and confirm the right shell.
    5. Pick "Custom path..." and enter a non discovered path; confirm a new pane uses it.
    6. Set a workspace override different from the app default; open a tab in that workspace; confirm the override wins.
    7. Restart the app; confirm the picks survived.

  **Files likely touched:** `src/ui/settings.rs`, `assets/styles.css` (if new CSS classes are needed).

  **Estimated scope:** M (1 to 2 files; rendering only, all logic landed in Task 8).

- [ ] **Task 10: Workspace context menu shell submenu**

  **Description:** The sidebar workspace context menu (`src/ui/sidebar.rs` and / or `src/ui/ctx_menu.rs`; locate during implementation) gains a "Shell" submenu listing `discover_installed()` results plus an "Use app default" entry. Picking a discovered shell dispatches `shell.set_workspace:<idx>:{ "program": "...", "args": [] }`; "Use app default" dispatches `shell.clear_workspace:<idx>`. No args editor in this menu (quick switcher only; the full editor is in Settings).

  **Acceptance criteria:**
  * Right clicking a workspace in the sidebar shows the existing context menu items plus a "Shell" submenu.
  * Submenu items are sourced from `discover_installed`.
  * The currently selected entry is marked active.
  * "Use app default" appears at the bottom of the submenu when an override is set; hidden otherwise.
  * Picking an entry dispatches the right command and the menu closes.
  * Unit test on the menu builder asserts the structural layout (label list + active marker).

  **Verification:**
  * `cargo test --lib ui::sidebar::` (or wherever the context menu builder lives) passes.
  * Manual: right click workspaces in the sidebar, confirm the submenu works end to end.

  **Files likely touched:** `src/ui/sidebar.rs` or `src/ui/ctx_menu.rs` (locate during implementation), possibly `assets/styles.css`.

  **Estimated scope:** M (1 to 2 files).

### Checkpoint after Phase 6
* The feature is end to end usable from both UI surfaces. Pause for human review before continuing.

### Phase 7: Cleanup

- [ ] **Task 11: Remove `SHELL=cmd.exe` bench override**

  **Description:** Delete `std::env::set_var("SHELL", "cmd.exe")` at `src/main.rs:325`. Bench mode now honors the configured `default_shell` like every other spawn. (Removing the General placeholder is part of Task 9; this task only handles the bench override and adds the regression test.)

  **Acceptance criteria:**
  * The `set_var("SHELL", ...)` call is gone from `main.rs`.
  * Regression test: a `#[test]` that grep / asserts the source string is absent (compile time match against `include_str!("main.rs")`).
  * `cargo run --features ... --bench scroll` (or the equivalent invocation) still completes without crashing on Windows. (Manual; bench numbers are not part of the acceptance criteria here.)

  **Verification:**
  * `cargo test --lib bench_no_set_shell` passes.
  * `cargo build --features bench` succeeds.

  **Files likely touched:** `src/main.rs`, possibly `src/bench/mod.rs` if a bench time default needs to be configured through the new path instead.

  **Estimated scope:** S (1 to 2 files).

### Final checkpoint
* All success criteria from `SPEC.md` verified.
* `cargo test`, `cargo clippy --all-targets --all-features`, `cargo fmt --check` clean.
* Manual smoke test of the full flow performed (both UI surfaces, both per workspace and app default).
* PR ready for review.

## Risks and mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| Old daemon clients see new wire field and break | High | Additive serde change with `#[serde(default)]` plus a "missing field deserializes to empty vec" test in Task 2. |
| One of 4 spawn sites missed; some tabs ignore the preference | High | Grep audit at the end of Task 5 (no `spawn_in.*None` literals outside tests); code review. |
| User args clash with daemon injected PowerShell `-NoExit -Command` | Medium | Regression test in Task 2 spawning PowerShell with both user args and a cwd; assert the cwd wins. |
| `discover_installed` slow on Windows (PATH walk on every settings open) | Low | Cap at 16 entries; only call when Settings → Shell is opened or the workspace context menu opens (not on startup). Cache inside `UiSnapshot` if it shows up in profiling. |
| Settings UI snapshot tests break on the new picker | Low | Update snapshots as the rendering lands; pure mechanical change. |
| Bench numbers shift after removing `SHELL=cmd.exe` override | Low | Documented; bench mode now uses configured shell. The hack was a workaround for a missing feature, which now exists. |
| Workspace context menu currently undiscovered location | Low | Task 10 begins with locating the existing right click menu builder via grep; small risk it lives in a less obvious file. |

## Notes on recent upstream changes

* `cbca91b` (style: settings restyle) switched the settings modal from `Display::Flex` to `Display::Grid` (168px nav | body) and widened it to 860px. Section builders are unchanged; Task 9 fits inside the body grid cell with no structural rewrites.
* `cfbfe44` (Ctrl+Arrow pane navigation) added 260 lines to `src/state.rs` but did not move any spawn site referenced in this plan; line numbers cited above (868, 1073, 1132) are post merge values.
