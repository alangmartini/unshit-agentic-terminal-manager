# Spec: Terminal-Scoped Notification System

## Assumptions
1. The primary target is the existing Rust terminal-manager desktop app on Windows, while keeping Unix builds compiling.
2. A notification is triggered from a process running inside a managed terminal, so the PTY daemon can provide workspace and pane metadata through environment variables.
3. The integration surface is a binary invocation: `terminal-manager notify --title <title> --text <text>`.
4. The default notification IPC endpoint is per-user/per-machine and can be overridden with `TM_NOTIFY_SOCKET`.
5. No new Cargo dependency should be added; desktop notifications use platform facilities best-effort.

## Objective
Build a notification path that lets Codex, Claude Code, or any child process inside a managed terminal notify the exact workspace and terminal pane that triggered the event.

Acceptance criteria:
- `terminal-manager notify --title "..." --text "..."` sends a notification request to the running terminal-manager process.
- When called from a managed terminal, the request automatically includes `TM_WORKSPACE_ID`, `TM_PANE_ID`, and `TM_NOTIFY_SOCKET`.
- The app shows a bottom-right card containing the notification title and text.
- Clicking the in-app card focuses the originating workspace and terminal.
- The app also emits a desktop notification; on Windows, clicking it sends an activation request back to the app and focuses the originating workspace and terminal.
- Activation requests wake and focus the terminal-manager window when supported by winit.

## Tech Stack
- Rust 2021
- `terminal-manager` app crate
- `unshit` framework with async subscriptions
- `unshit-ptyd` daemon and existing named-pipe / Unix-socket transport
- `serde` / `serde_json` for notification IPC payloads
- Existing CSS and toast UI surface for bottom-right cards

## Commands
- Build: `cargo build -p terminal-manager`
- Unit tests: `cargo test -p terminal-manager notifications`
- PTY daemon tests: `cargo test -p unshit-ptyd session_env`
- Focused UI toast tests: `cargo test -p terminal-manager ui::toasts`
- Format check: `cargo fmt --check`

## Project Structure
- `src/notifications.rs` -> notification protocol, CLI parser/sender, IPC subscription helpers, desktop notification helper
- `src/main.rs` -> CLI routing, notification subscription registration, external activation handling
- `src/state.rs` -> targeted notification toast metadata and focus mutation
- `src/ui/toasts.rs` -> bottom-right titled notification card rendering
- `assets/styles.css` -> notification card styling
- `crates/unshit-ptyd/src/session/mod.rs` -> PTY child environment variables
- `crates/unshit-framework/crates/unshit-app/src/event_sink.rs` and `app.rs` -> app window activation event
- `specs/notification-system.md` -> this living spec

## Code Style
Use small typed structs for wire data and keep string parsing at the process boundary.

```rust
let request = NotificationIpcRequest::Notify {
    title,
    text,
    workspace_id,
    pane_id,
};
send_notification_request(&socket, &request).await?;
```

Conventions:
- Prefix notification environment variables with `TM_`.
- Keep IPC payloads JSON and version-tolerant with `serde(default)` for optional fields.
- Keep UI mutations inside `state.rs`; the IPC layer should call focused helpers rather than rewriting state directly.

## Testing Strategy
- Unit-test CLI parsing and environment fallback behavior.
- Unit-test notification socket path override behavior.
- Unit-test PTY child environment construction.
- Unit-test targeted notification insertion and activation focus behavior.
- Unit-test UI card rendering for title and body.
- Run focused crate tests plus formatting/build checks.

## Boundaries
- Always: preserve existing toast error behavior and auto-dismiss behavior.
- Always: keep notification IPC local to the machine/user and avoid network listeners.
- Always: treat desktop notifications as best-effort; the in-app card is authoritative.
- Ask first: adding a Cargo dependency for native toast APIs.
- Ask first: changing the PTY daemon protocol wire shape for existing session lifecycle requests.
- Never: block the render thread on notification IPC or desktop notification helpers.
- Never: remove existing workspace/pane focus semantics.

## Implementation Plan
1. Add a notification protocol module with CLI parsing, env fallbacks, default socket path, local IPC send/receive helpers, and Windows desktop balloon helper.
2. Add a notification subscription that binds the local IPC endpoint and yields rebuild/window-activation events after mutating shared state.
3. Extend PTY session spawn environment with `TM_WORKSPACE_ID`, `TM_PANE_ID`, and `TM_NOTIFY_SOCKET`.
4. Extend toast state/view rendering to support optional title and workspace/pane target metadata.
5. Add app window activation event support to the framework event loop.
6. Verify with focused unit tests, `cargo fmt --check`, and `cargo build -p terminal-manager`.

## Tasks
- [ ] Add spec and notification module.
  - Acceptance: CLI parser accepts notify/activate and falls back to `TM_*` env vars.
  - Verify: `cargo test -p terminal-manager notifications`.
  - Files: `specs/notification-system.md`, `src/notifications.rs`, `src/main.rs`
- [ ] Add IPC subscription and activation event.
  - Acceptance: app can mutate state from local notification requests and focus window on activation.
  - Verify: focused unit tests and build.
  - Files: `src/bridge.rs`, `src/notifications.rs`, `crates/unshit-framework/crates/unshit-app/src/event_sink.rs`, `crates/unshit-framework/crates/unshit-app/src/app.rs`
- [ ] Add targeted titled cards.
  - Acceptance: notification cards render title and text and click focuses target.
  - Verify: `cargo test -p terminal-manager ui::toasts`.
  - Files: `src/state.rs`, `src/ui/toasts.rs`, `assets/styles.css`
- [ ] Add PTY environment injection.
  - Acceptance: spawned shells receive notification socket, workspace id, and pane id.
  - Verify: `cargo test -p unshit-ptyd session_env`.
  - Files: `crates/unshit-ptyd/src/session/mod.rs`

## Success Criteria
- A managed terminal can run `terminal-manager notify --title done --text "agent finished"` and the app shows a titled bottom-right notification for that pane.
- Clicking the in-app card switches to the originating workspace and pane.
- On Windows, clicking the desktop notification activates the same target in the app.
- No new dependency is introduced.
- Focused tests and build pass.

## Open Questions
- Whether a future release should add a native Windows notification dependency for full Windows toast integration instead of the no-dependency tray balloon helper.
