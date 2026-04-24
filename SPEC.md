# SPEC: Terminal process persistence (tmux-style daemon)

Status: approved
Owner: Alan
Tracking: https://github.com/alangmartini/unshit-agentic-terminal-manager/issues/119
Branch: feat/tmux-daemon-persistence

## 1. Objective

Make terminal processes outlive the terminal-manager UI. Closing the app (or having it crash) must not kill the shells running inside it. Reopening the app must silently reattach to every running shell with its scrollback and cursor state intact. Give the user explicit controls to kill terminals, singly, per workspace, or everywhere, and make sure one shell crashing or one pane's emulator panicking cannot take any other terminal with it.

### Target user

Power user running multiple long-lived shells (build servers, watchers, REPLs, SSH tails) across workspaces. Currently every quit of terminal-manager kills everything. The persistence story has to feel like tmux: "I can close the UI, reboot the UI, and come back to exactly what I had."

### Non-goals for MVP

- Persistence across machine reboot. The daemon dies with the machine, which kills every child shell. Revisit later via "named sessions that auto-respawn on daemon start."
- Remote attach over SSH or network.
- Sharing a session between two concurrent UI clients.
- Replaying command history or shell state beyond the cell grid and scrollback that tmux itself preserves.

## 2. High-level architecture

A separate long-running background process (`unshit-ptyd`) owns every PTY master, every child shell, and every VTE parser / CellGrid. The UI app (terminal-manager) becomes a thin client that connects to the daemon over local IPC, streams output, and forwards keyboard input. The daemon has no UI and no frontend dependencies.

```
+----------------------+          IPC           +----------------------+
|  terminal-manager    | <-- named pipe / ----> |   unshit-ptyd        |
|  (UI client)         |     unix socket        |   (daemon)           |
|                      |                        |                      |
|  - renders CellGrid  |                        |  - owns PtyManager   |
|  - captures keys     |                        |  - owns VTE parsers  |
|  - local state only  |                        |  - owns CellGrids    |
+----------------------+                        |  - owns scrollback   |
                                                +----------------------+
                                                         |
                                                         v
                                                   child shells
```

### Why a daemon

Without a second process owning the PTY master, closing the app closes the master file descriptor, which sends SIGHUP to every child shell on Unix and effectively kills the ConPTY session on Windows. There is no "orphan the PTY" trick. A real tmux-style split is the only design that delivers the user's requirement.

### Daemon lifecycle

- **Spawn:** the UI tries to connect to the daemon on startup. If the connection fails, the UI spawns `unshit-ptyd` as a detached process, then retries the connection.
- **Shutdown:** the daemon keeps running after the UI exits as long as at least one session is alive. If a shutdown request arrives with zero sessions, or the "kill all and quit" command runs, the daemon exits cleanly.
- **Single instance:** the daemon holds an exclusive lock on its socket / pipe path. A second spawn attempt detects the lock and exits without error.

## 3. Core features and acceptance criteria

### F1. Terminals survive app close

> Close the app while three shells are running. Reopen the app. All three shells are running, same scrollback, same cursor position, same workspace layout.

Acceptance:
- Graceful app close without confirmation-to-kill leaves every child shell alive in the daemon.
- Reopening auto-attaches to every prior session silently. No "resume?" prompt.
- For each reattached terminal the user sees the pre-close grid contents on the first frame and the scrollback buffer is available to scroll into.
- The prior workspace layout (tab order, split layout, active tab, active pane) is restored exactly.

### F2. Terminals survive UI crash

> Force-kill the terminal-manager process. Launch it again. All shells are still running.

Acceptance:
- `taskkill /F /PID <ui-pid>` (Windows) or `kill -9 <ui-pid>` (Unix) on the UI does not kill `unshit-ptyd` or any child.
- Relaunching the UI reattaches as in F1.

### F3. Scrollback and grid state are preserved

Acceptance:
- The daemon parses PTY output itself (owns a VTE parser and CellGrid per session) so it always has authoritative current-frame state.
- On attach the daemon sends the client a snapshot: full CellGrid contents, cursor position, cursor visibility, scrollback lines (up to a bounded cap, default 10,000 lines).
- After snapshot, the daemon streams live byte chunks as before.

### F4. One shell or one pane cannot crash the others

Acceptance:
- A shell exiting (`exit` / process crash) only ends that session. Other sessions and the daemon itself stay alive.
- A panic inside one session's VTE parser thread only kills that session (the daemon catches the panic, marks the session as errored, notifies clients, keeps serving others).
- The UI does not use `.expect()` or `.unwrap()` on per-pane state in paths that can be reached from any pane's byte stream. Specifically the render closure, the state mutex, and the per-terminal mutex must not poison each other's paths.

### F5. Per-workspace kill

> Right-click a workspace in the sidebar, pick "Kill all terminals in workspace".

Acceptance:
- New entry in the workspace sidebar context menu: "Kill all terminals in workspace".
- Invoking it kills every session tagged with that workspace's id in the daemon and removes the corresponding panes from the UI.
- Workspace itself is not deleted, just emptied.
- Confirmation dialog before killing.

### F6. Kill everything

> Click "Kill all terminals" in the app menu.

Acceptance:
- New entry in the app menu (not a titlebar button): "Kill all terminals".
- Invoking it kills every session across every workspace.
- Confirmation dialog before killing.

### F7. Close-app prompt with "remember my choice"

> When closing the app with live terminals, offer three choices.

Acceptance:
- On close, if at least one session is alive and "remember my choice" has not been set, show a modal with three buttons: "Keep running", "Kill all and quit", "Cancel".
- A checkbox "Remember my choice" persists the non-Cancel selection to settings. Next close applies the remembered choice silently.
- The remembered choice is visible and resettable in the settings modal.
- Cancel never gets remembered.

### F8. Named sessions with UI (stretch, P1, still MVP)

> Give a session a name, see it in a list, attach or kill it.

Acceptance:
- A session can be given a name via right-click context menu on the tab: "Name this session".
- A new "Sessions" panel (surface TBD: sidebar section or settings-modal tab) lists all named sessions with their workspace, tab title, and alive/dead status.
- From the panel the user can attach a named session into the current workspace, rename it, or kill it.
- Unnamed sessions show as `<shell> (<pid>)` and are still listed.

If this turns out to be a lot of UI work we descope to a plain list in the settings modal for MVP and defer the sidebar integration.

## 4. IPC protocol

Transport:
- Windows: named pipe, e.g. `\\.\pipe\unshit-ptyd-<user-sid>`.
- Unix: unix domain socket at `$XDG_RUNTIME_DIR/unshit-ptyd.sock` (or `/tmp/unshit-ptyd-<uid>.sock` as fallback).

Framing: length-prefixed JSON for control messages, length-prefixed raw bytes for PTY output chunks. Each frame is `u32 length | u8 kind | payload`. `kind = 0` means JSON control; `kind = 1` means binary output chunk with header `{session_id: u64}` packed as `u64` prefix. We will revisit if benchmarks show JSON overhead matters; for v1 keep it simple.

Control messages (request / response):

| Request | Payload | Response |
|---|---|---|
| `hello` | `{client_version}` | `{server_version, protocol_version}` |
| `list_sessions` | `{}` | `{sessions: [{id, name?, workspace_id, cols, rows, alive, pid, shell}]}` |
| `spawn_session` | `{workspace_id, cols, rows, cwd?, shell?, name?}` | `{session_id}` |
| `attach_session` | `{session_id}` | `{snapshot: {grid_cells, cursor, scrollback_lines}}` |
| `detach_session` | `{session_id}` | `{ok}` |
| `write` | `{session_id, bytes_len}` + raw bytes | (no response) |
| `resize` | `{session_id, cols, rows}` | `{ok}` |
| `kill_session` | `{session_id}` | `{ok}` |
| `kill_workspace` | `{workspace_id}` | `{killed_ids: [...]}` |
| `kill_all` | `{}` | `{killed_ids: [...]}` |
| `rename_session` | `{session_id, name}` | `{ok}` |
| `shutdown` | `{}` | `{ok}` (only if no sessions alive) |

Events (daemon-initiated, pushed to attached clients):

| Event | Payload |
|---|---|
| `output` | `{session_id}` + raw bytes |
| `session_exited` | `{session_id, exit_code, signal?}` |
| `session_crashed` | `{session_id, reason}` |

All control messages are size-capped (max 1 MiB) and have a per-connection timeout to prevent a misbehaving client wedging the daemon.

## 5. Persisted state

Add a sessions manifest next to the existing `workspaces.json`:

```
%APPDATA%/com.godly.terminal/
  workspaces.json        # existing, unchanged schema
  sessions.json          # NEW: session id -> workspace / tab / pane mapping + name
  close-preference.json  # NEW: remembered "keep / kill" choice, optional
```

`sessions.json` is maintained by the **daemon**, not the UI, so it survives UI crashes. It is the single source of truth for "which sessions exist, and where do they belong in the layout." The UI reads it on attach and uses it to reconstruct the pane layout. Schema sketch:

```json
{
  "sessions": [
    {
      "id": 7,
      "name": "web-dev",
      "workspace_id": 2,
      "tab_id": 3,
      "pane_id": 5,
      "cols": 120,
      "rows": 40,
      "cwd": "C:/Users/alanm/project",
      "shell": "powershell.exe",
      "created_at": "2026-04-22T14:00:00Z"
    }
  ]
}
```

Daemon writes this atomically (write to `sessions.json.tmp`, rename) on every session create / kill / rename / pane-move.

## 6. Project structure

```
crates/
  unshit-framework/          # existing, untouched
  unshit-ptyd/               # NEW daemon crate
    Cargo.toml
    src/
      main.rs                # daemon entry, arg parsing, lockfile, event loop
      ipc/
        mod.rs               # IPC abstraction
        pipe_windows.rs      # Windows named pipe server
        socket_unix.rs       # Unix socket server
        protocol.rs          # message types, serialization, framing
      session/
        mod.rs               # Session struct: pid, pty, parser, grid, scrollback
        manifest.rs          # sessions.json read/write
        scrollback.rs        # bounded ring buffer of scrollback lines
      pty/
        mod.rs               # PTY wrapper (moved from src/pty)
src/                         # existing UI crate
  ipc/
    mod.rs                   # client-side IPC (connect, reconnect, retry)
    protocol.rs              # shared message types; import from unshit-ptyd if feasible
  pty/
    mod.rs                   # STAYS for now as "talk to daemon" facade; or gets deleted if the bridge layer does everything
  bridge.rs                  # adapted to pull bytes from IPC instead of in-process PTY reader
  state.rs                   # adapted: terminals map keyed by daemon session_id
  persist.rs                 # close preference added; workspaces still here
  ui/
    sidebar.rs               # + "Kill all in workspace" context menu entry
    app_menu.rs              # + "Kill all terminals" entry (create file if missing)
    sessions_panel.rs        # NEW: named sessions UI
    close_prompt.rs          # NEW: three-option modal
tests/                       # integration tests
  daemon_lifecycle.rs        # spawn, connect, shutdown with no sessions
  session_persistence.rs     # survive UI close / crash
  protocol_framing.rs        # malformed frames, size caps, timeout
  crash_isolation.rs         # panic in one session, others survive
SPEC.md                      # this file
```

Shared types (protocol message structs) live in `unshit-ptyd/src/ipc/protocol.rs` and the UI depends on the daemon crate only for that module (library target). Alternatively they can live in a third `unshit-ipc` crate if the coupling feels wrong; decide during implementation.

## 7. Commands

```bash
# build everything (daemon + UI)
cargo build

# run the UI (auto-spawns the daemon if missing)
cargo run

# run only the daemon (for debugging)
cargo run -p unshit-ptyd

# health check (CLI flag on the daemon)
cargo run -p unshit-ptyd -- --status

# stop the daemon (only succeeds when zero sessions alive)
cargo run -p unshit-ptyd -- --shutdown

# nuke the daemon (for dev)
cargo run -p unshit-ptyd -- --kill-all-and-shutdown

# all tests
cargo test
cargo test -p unshit-ptyd        # daemon only
cargo test --test '*'            # integration tests
cargo llvm-cov --html            # coverage
```

## 8. Code style

Same as the rest of the project (from CLAUDE.md):

- TDD. Red / green / refactor. Every `feat:` and `fix:` commit ships with tests.
- Conventional commits: `feat(ptyd):`, `feat(ipc):`, `feat(ui):`, etc.
- No em dashes anywhere (per user's global CLAUDE.md).
- No Claude attribution in commits.
- No em-dash fallbacks in docs or UI copy.
- Do not remove comments unless the underlying constraint is demonstrably gone.
- No new abstractions until a second concrete use-case shows up. The `Session` struct and the IPC protocol are the only new abstractions for v1.
- Errors use `thiserror` enums at crate boundaries; internal code can use `Result<_, io::Error>` and newtype at the seam.
- The daemon must never panic out of its main loop. Per-session workers are allowed to panic; the supervisor catches and reports.

## 9. Testing strategy

### Unit tests (per module)

- `ipc::protocol`: round-trip encode / decode, malformed frame rejection, size-cap enforcement.
- `session::scrollback`: bounded ring buffer, correct eviction, snapshot window selection.
- `session::manifest`: atomic write, crash-safe read (partial write ignored), schema migration.
- `pty`: reuse existing tests from `src/pty/mod.rs`, move them into the daemon crate.
- Close-prompt remembered-choice persistence.

### Integration tests (`tests/`)

- `daemon_lifecycle`: spawn daemon in background, connect, `hello`, `shutdown`. Cover lock contention (second spawn exits clean).
- `session_persistence`: spawn session, write bytes, detach client, reconnect, confirm snapshot equals prior state. Scrollback preserved.
- `protocol_framing`: oversize frame rejected, slow client drained and disconnected, split frames reassembled.
- `crash_isolation`: two sessions, inject a synthetic panic in one session's parser thread, confirm the other keeps streaming.
- `kill_workspace` and `kill_all`: confirm only the intended sessions die.

### End-to-end / regression tests

- **Regression F1:** close app, reopen, assert scrollback matches. Requires a test harness that can spawn the UI headless, drive PTY input, close, relaunch. If the harness is too expensive, skip in CI and run manually; document in the PR.
- **Regression F2:** `taskkill /F` (or `kill -9`) the UI mid-session, relaunch, confirm daemon + sessions alive. Platform-gated.

### Visual verification (manual, required by user)

Per the user's memory rule, visual regressions are verified by the user. Do not automate screenshots. The PR checklist will include:
- [ ] User has run `cargo run`, closed, reopened, verified three shells reattach.
- [ ] User has crashed the UI with taskkill, relaunched, verified reattach.
- [ ] User has confirmed the close-prompt modal works and remembering the choice works.
- [ ] User has confirmed "Kill all in workspace" and "Kill all terminals" work.

### Coverage gate

- `cargo llvm-cov` must not drop. Every new module ships with tests as noted above. Manifest, scrollback, IPC protocol, and close-prompt are fully unit-testable without the daemon running.

## 10. Boundaries

### Always do

- Keep the daemon crate dependency-free from the UI crate. The daemon must build and run without any unshit-framework, wgpu, or taffy code.
- Write `sessions.json` atomically (write-temp + rename).
- Cap scrollback per session (default 10,000 lines, configurable in settings).
- Cap in-flight bytes per client (back-pressure): slow clients get dropped chunks rather than OOMing the daemon.
- Respect Windows first in all platform-specific code paths and tests. Mac / Linux parity is secondary.
- Convert every `.expect()` and `.unwrap()` in the bridge and render path to a handled error or an explicit "this-pane-only" failure mode. Review state.rs and bridge.rs as part of F4.

### Ask first

- Any change to the on-disk schema (`sessions.json`, `close-preference.json`) after v1 ships.
- Any protocol change that isn't additive.
- Any new daemon-side dependency larger than 200 kLOC of transitive impact.
- Scope expansion toward machine-reboot persistence, SSH attach, or multi-client sharing.
- Dropping the named-sessions UI from MVP if it balloons.

### Never do

- Never kill a session without explicit user action or explicit user setting. "Cleanup" and "orphan reaping" are not user-initiated actions.
- Never add Claude as co-author on commits (per CLAUDE.md).
- Never skip the Red / Green / Refactor cycle for a bug fix. Every fix lands with the reproducing test.
- Never use em dashes in code comments, docs, UI copy, or commit messages (per user's global CLAUDE.md).
- Never panic out of the daemon main loop. Use `std::panic::catch_unwind` around session worker entry points.
- Never read `.pen` files in this project directly; not relevant to this spec but called out because the user has a pencil MCP attached.
- Never automate screenshots, cursor moves, or foreground window capture as part of verification (per user's memory rules).

## 11. Rollout plan

Suggested slice order, each slice lands as its own PR with tests passing:

1. **Scaffold.** Add `crates/unshit-ptyd`, workspace member, empty main, basic `--status` flag.
2. **IPC transport.** Named pipe (Windows) and unix socket (Unix) server + client, framing, `hello` + `shutdown` only. Full unit + integration tests.
3. **Session lifecycle (no persistence yet).** `spawn_session`, `write`, `resize`, `kill_session`, `output` events. Move `src/pty` into the daemon. UI talks to daemon instead of in-process PTY. App behavior unchanged at this point.
4. **Scrollback + snapshot on attach.** Daemon owns VTE + CellGrid. `attach_session` returns a snapshot. Client renders it. UI close still kills sessions.
5. **Close survival.** Detach instead of kill on graceful close. Reattach on startup. `sessions.json` manifest.
6. **Crash isolation pass.** Audit bridge.rs / state.rs for `.expect()` / `.unwrap()`. `catch_unwind` around session workers. Tests.
7. **UI controls.** Kill-all, kill-workspace, close-prompt with remembered choice.
8. **Named sessions.** Context menu rename + sessions panel.
9. **Polish.** Error copy, coverage, docs, CHANGELOG fragments.

## 12. Open questions (resolve before implementation)

- **Daemon binary location in dev vs release.** In `cargo run`, the UI can locate the daemon via `CARGO_BIN_EXE_unshit-ptyd`. In an installed build the daemon binary needs to live next to the UI executable. Decide path resolution strategy (probably: same dir as current exe, fall back to `PATH`).
- **Daemon logging.** Where do logs go? File in `%APPDATA%/com.godly.terminal/logs/ptyd.log` with rotation? Stderr only? I lean "file + rotation" since the daemon has no terminal.
- **Scrollback format on disk.** Not persisted to disk in v1; lives in RAM in the daemon. If the daemon is told to hibernate (future) we'd need a format.
- **Shell per-session vs per-app.** Currently the shell is chosen from `$SHELL` at spawn time in the UI. The daemon takes over, so the UI has to pass the desired shell in `spawn_session`. Confirm the UI passes `$SHELL` unchanged.
- **Resize storm.** Renderer can emit rapid resize events while the user drags. Debounce in the UI before sending to the daemon, as before.

---

End of spec. Please review and either approve, change, or ask for more detail. Once approved I'll create an implementation tracking issue and start on slice 1 (scaffold the daemon crate).
