# Spec: Configurable default shell

## Objective

Let the user choose which shell program (PowerShell, pwsh, Git Bash, cmd, bash, zsh, etc.) is launched when a new terminal is spawned. Today the daemon always falls back to `pty::default_shell()` (`$SHELL` env var, then `powershell.exe` on Windows / `bash` on Unix) because the UI never fills the protocol's `shell` field.

Users want two levels of control:

1. **App wide default**: one shell used by every new terminal across every workspace.
2. **Per workspace override**: a workspace may pin its own shell (e.g. a Linux project workspace pinned to `wsl.exe`, an admin workspace pinned to `pwsh`).

A pane that is reattaching to a surviving daemon session keeps whatever shell it was originally spawned with; the preference only affects fresh spawns.

### User stories

* As a Windows developer, I want new terminals to open in `pwsh` (PowerShell 7) instead of `powershell.exe`, without setting `$env:SHELL` globally.
* As a multi project user, I want my "infra" workspace to default to `wsl.exe -d Ubuntu` while my "windows app" workspace defaults to `pwsh`.
* As a power user, I want to pass args (e.g. `bash --login -i`, `wsl.exe -d Ubuntu`) and have them survive across restarts.

### Non goals

* Per pane shell override at spawn time (the user can change the workspace setting and open a new pane).
* Restarting or reattaching existing sessions when the preference changes.
* A shell launcher menu (separate from the default) that lets the user pick a shell ad hoc when opening a new terminal. Worth a follow up; out of scope here.
* Wiring the MCP tools (`get_default_shell`, `set_default_shell`, `list_available_shells`) to the new state. Those names appear in the registered tool list but are not implemented today; out of scope for this spec.

## Tech Stack

* Rust 2021, existing crate layout. No new third party crates.
* `portable_pty` 0.8 (already used) handles spawning programs with args.
* `serde` + `serde_json` for persistence (already used).
* `dirs` for home / config dir resolution (already used).
* IPC remains length prefixed JSON over a Windows named pipe / Unix socket. The `Request::SpawnSession` variant gains a `shell_args` field (additive, defaults to empty vector, no protocol version bump per the contract in `protocol/message.rs`).

## Commands

```
Build:           cargo build
Type check:      cargo check
Run app:         cargo run
Unit tests:      cargo test --lib
Integration:     cargo test --test '*'
All tests:       cargo test
Lint:            cargo clippy
Format check:    cargo fmt --check
Coverage HTML:   cargo llvm-cov --html
```

Every commit must pass `cargo test`, `cargo clippy`, `cargo fmt --check` per project CLAUDE.md.

## Project Structure

The change touches the existing tree; no new top level directories.

```
src/
  shell.rs                 NEW. ShellSpec type, resolution, discovery, MCP glue.
  state.rs                 +ShellSpec on AppState, dispatch handlers, UiSnapshot fields.
  persist.rs               +default_shell + per workspace shell in PersistedState.
  pty.rs                   spawn_in / attach_or_spawn carry ShellSpec to the daemon.
  ui/
    settings.rs            Replace placeholder; build real shell picker in Shell tab.
  main.rs                  Resolve effective shell on initial spawn; remove bench override.
crates/unshit-ptyd/
  src/protocol/message.rs  +shell_args field on SpawnSession (additive).
  src/client/mod.rs        spawn_session signature gains shell_args.
  src/session/mod.rs       Session::spawn forwards shell_args to CommandBuilder.
  src/pty/mod.rs           default_shell stays as the last resort fallback.
  tests/                   +shell propagation integration test.
crates/unshit-framework/   No changes expected.
SPEC.md                    This document.
```

The Shell setting is reachable from two UI surfaces: Settings → Shell tab (full editor) and the workspace context menu in the sidebar (quick switcher: pick a discovered shell, no editing).

## Code Style

A single struct describes a shell: program plus args. PowerShell cwd quirk handling stays in the daemon and is invisible to callers.

```rust
// src/shell.rs

use std::path::{Path, PathBuf};

/// A shell program plus its launch args. Stored in `workspaces.json` and
/// passed across the IPC boundary as `(shell, shell_args)`.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ShellSpec {
    /// Absolute path or PATH lookup name. Empty string means "fall back".
    pub program: String,
    /// Args forwarded to the program before any daemon side cwd args.
    #[serde(default)]
    pub args: Vec<String>,
}

impl ShellSpec {
    pub fn is_empty(&self) -> bool {
        self.program.is_empty()
    }
}

/// Resolve which shell a pane should spawn with. Workspace override wins
/// over the app wide default; both are skipped when their `program` is
/// empty so that "unset" round trips cleanly through the JSON config.
/// Returns `None` to mean "let the daemon pick `default_shell()`", which
/// preserves today's behavior for users who never touch the setting.
pub fn resolve(
    workspace_default: Option<&ShellSpec>,
    app_default: Option<&ShellSpec>,
) -> Option<ShellSpec> {
    workspace_default
        .filter(|s| !s.is_empty())
        .or(app_default.filter(|s| !s.is_empty()))
        .cloned()
}

/// Best effort scan of installed shells. Returns absolute paths in
/// stable order. PATH stems plus a small set of known Windows install
/// locations (Git for Windows, WSL launcher) and Unix shells. Used to
/// populate the picker; never blocks startup, never errors out.
pub fn discover_installed() -> Vec<PathBuf> {
    // ...PATH walk for known stems + platform specific well known paths.
}
```

Conventions:

* No em dashes in source comments or docs (matches user CLAUDE.md).
* Public functions get a one line `///` doc explaining intent, not mechanics.
* Tests in the same file under `#[cfg(test)] mod tests`, one behavior per test, regression tests reference the issue number in a comment per CLAUDE.md.
* No new error handling for "can't happen" branches; trust internal invariants per project CLAUDE.md.

## Testing Strategy

Strict TDD, Red, Green, Refactor (project mandate). Every behavior described below is captured by at least one test before code is written.

### Unit tests

* `src/shell.rs`
  * `resolve` returns workspace override when both set.
  * `resolve` falls back to app default when workspace override is empty.
  * `resolve` returns `None` when both unset (preserves today's behavior).
  * `discover_installed` returns at least one entry on the test host (sanity).
  * `ShellSpec` round trips through serde with empty args defaulted in.
* `src/persist.rs`
  * Loading an old `workspaces.json` (no shell fields) yields default `ShellSpec::default()` everywhere.
  * Saving + reloading preserves both `default_shell` and per workspace `shell`.
* `src/state.rs`
  * Dispatching `shell.set_default:<json>` updates `AppState.default_shell`.
  * Dispatching `shell.set_workspace:<idx>:<json>` updates the right workspace.
  * Dispatching `shell.clear_default` sets the program back to empty.
  * The chosen `ShellSpec` is reflected in `UiSnapshot` so the Settings UI redraws.

### Daemon protocol tests

* `crates/unshit-ptyd/src/protocol/message.rs`
  * `SpawnSession` round trips with `shell_args` set and unset.
  * Old wire format (no `shell_args`) deserializes with an empty vector.
* `crates/unshit-ptyd/tests/ipc_session_spawn.rs`
  * Spawning with explicit `shell` + `shell_args` reaches `CommandBuilder` (use `cmd.exe /C echo` on Windows, `/bin/sh -c echo` on Unix and assert the echoed token shows up in the byte stream).

### Integration tests

* `tests/shell_resolution_through_spawn.rs` (new)
  * Set app default, no workspace override, spawn a pane: daemon receives the app default.
  * Set workspace override, spawn a pane in that workspace: daemon receives the override.
  * Clear both, spawn: daemon receives `None` (today's behavior).

### Regression / known issue tests

* `bench mode no longer overrides $SHELL`: the previous code in `main.rs:325` force set `$SHELL=cmd.exe` on Windows under `--bench`. The new code path must not touch the env var; instead the bench config selects a shell deterministically through the same preference. A test asserts no `set_var("SHELL", _)` remains in `main.rs`.

### Coverage

* `cargo llvm-cov` after the work is done. New `shell.rs` and the resolution path must hit all branches; per project CLAUDE.md, "test coverage must not decrease."

### What is intentionally not covered

* Visual regression of the Settings panel itself (no harness for it). Manual verification via `cargo run` is required per CLAUDE.md, "tests passing does NOT mean the app works correctly."

## Boundaries

### Always

* Run `cargo test`, `cargo clippy`, `cargo fmt --check` before committing.
* Write the failing test first (Red, Green, Refactor) for both features and bug fixes.
* When a fix touches the protocol, keep the wire change additive and document the deserialization fallback.
* Honor the existing PowerShell cwd quirk: when the resolved program looks like `pwsh` / `powershell`, the daemon still appends `-NoExit -Command "Set-Location ..."` after the user's args.
* Persist preference changes immediately (same trigger as workspace edits do today).

### Ask first

* Adding new dependencies (none expected; `dirs`, `serde`, `portable_pty` already present).
* Bumping the daemon `PROTOCOL_VERSION` (the `shell_args` change is additive and must not require a bump).
* Changing the on disk file name or location for `workspaces.json` (a future migration is fine, not in this spec).
* Removing or renaming any existing `ShellIntegration` toggle / `Shell` settings tab field.
* Wiring a per pane shell override (out of scope; opens design questions).

### Never

* Restart or reattach existing sessions when the user changes the default; only future spawns are affected.
* Delete or rewrite the daemon side `default_shell()` fallback. It stays as the last resort when nothing is configured and `$SHELL` is unset.
* Hard code shell paths anywhere outside `src/shell.rs::discover_installed`.
* Embed shell args in the `program` string. Args go in `ShellSpec::args`.
* Skip the manual `cargo run` smoke test before opening the PR.

## Success Criteria

A change that satisfies all of the following:

1. Settings → Shell tab has an "App default" picker: a dropdown of `discover_installed()` results plus a "Custom path..." entry that reveals an editable program field, plus an always visible args field. Selecting a discovered entry fills the program field and clears the custom slot.
2. The same Shell tab has a "Per workspace override" subsection with one row per workspace; each row uses the same dropdown / custom / args layout. An empty program means "use app default."
3. The workspace context menu in the sidebar gains a "Shell" submenu that lists discovered shells; picking one sets the per workspace override (program only, no args). "Use app default" clears the override.
4. Changing either the app default or a workspace override (from either UI surface) updates `workspaces.json` immediately.
5. Opening a new tab in a workspace with an override spawns the override shell. Opening a new tab in any other workspace spawns the app default. With both unset, the daemon falls back to today's `default_shell()` behavior unchanged.
6. Restarting the app preserves the picked shell on every new spawn.
7. `cargo test`, `cargo clippy --all-targets --all-features`, `cargo fmt --check` all pass with no warnings.
8. Manual smoke test: launch the app, change the shell to a value different from the platform default, open a new pane, type `echo $0` (or `$PSVersionTable.PSVersion` on PowerShell, `$Host.Name` on pwsh) and confirm the right shell is running.
9. The `SHELL=cmd.exe` override at `main.rs:325` is gone; bench mode honors the configured shell.
10. The placeholder "Default shell" row in Settings → General is removed (the real picker lives in the Shell tab).
11. A user with no `workspaces.json` (fresh install) sees the picker pre populated: `pwsh.exe` if discovered, else `powershell.exe`, else today's `default_shell()` value. Resolution still treats an empty program as "fall back."

## Decisions resolved during planning

* **Picker shape:** dropdown of discovered shells, with a "Custom path..." entry that reveals a text input plus a separate always visible args field.
* **UI surfaces:** both Settings → Shell tab (full editor) and the workspace context menu (quick switcher; per workspace overrides only, no args).
* **First time inference:** pre populate `default_shell.program` on fresh install with `pwsh.exe` > `powershell.exe` > `default_shell()` value.
* **MCP wiring:** dropped from this spec. The MCP tool names exist in the registered tool list but are not implemented today.
