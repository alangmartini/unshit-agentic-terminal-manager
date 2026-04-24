# TODO: Issue #130 (RPC error toasts)

Tracks: [`specs/130-rpc-error-toasts.md`](../specs/130-rpc-error-toasts.md)
Plan: [`plan.md`](plan.md)

Mark tasks done in order. Do not skip a checkpoint.

## Slice A: Foundation

### A1. Framework toast module
- [ ] Red: write 6 unit tests in `crates/unshit-framework/crates/unshit-core/src/toast.rs`
- [ ] Green: implement `Toast`, `ToastKind::Error`, `ToastStore`
- [ ] Re-export from `crates/unshit-core/src/lib.rs` (not in `prelude`)
- [ ] `cargo test -p unshit-core toast` green
- [ ] `cargo clippy -p unshit-core --all-targets -- -D warnings` clean
- [ ] Commit: `feat(framework): toast notification primitive`

### A2. App state plumbing
- [ ] Verify `ElementDef` supports `role` / `aria-*` attributes (framework probe)
- [ ] Red: 3 new tests in `src/state.rs::tests`
- [ ] Add `AppState.toasts: ToastStore`, init via `with_capacity(3, 8)` at every construction site
- [ ] Add `AppState.sessions_stale: bool`
- [ ] Add `ToastView` struct in `state.rs`
- [ ] Add `UiSnapshot.toasts` and `UiSnapshot.sessions_stale`, populate in `ui_snapshot()`
- [ ] Add `pub fn push_error_toast(state, msg)`
- [ ] Add dispatch arm `toast.dismiss:<id>`
- [ ] All existing tests stay green
- [ ] Commit: `feat(state): toast store, dismiss dispatch, sessions_stale flag`

### A3. UI overlay, root wiring, CSS, bridge tick
- [ ] Red: 4 builder tests in new `src/ui/toasts.rs`
- [ ] Implement `build_toast_overlay(snap, shared)` with `role=status`, `aria-live=polite`, `aria-atomic=false`
- [ ] Add `pub mod toasts;` to `src/ui/mod.rs`
- [ ] Wire into root tree in `src/main.rs` after `build_confirm_dialog_overlay`
- [ ] Add CSS rules: `.toast-overlay`, `.toast`, `.toast-error`, `.toast-overlay-hidden`
- [ ] Hook `state.toasts.advance_ticks(1)` into `cursor_blink_subscription` in `src/bridge.rs`
- [ ] `cargo run` launches without panic; no toasts visible
- [ ] Commit: `feat(ui): toast overlay, root wiring, blink-driven dismiss`

**Checkpoint after A3:** smoke check with user. Approve to continue.

## Slice B: Refresh failure

- [ ] Red: `refresh_sessions_failure_sets_stale_and_pushes_toast` in `src/state.rs`
- [ ] Red: `sessions_section_renders_stale_chip_when_flag_set` in `src/ui/settings.rs`
- [ ] Modify `refresh_sessions`: `Err` sets stale + pushes toast; `Ok` clears stale
- [ ] Modify `build_sessions_section`: append `.sessions-refresh-stale` chip when flag set
- [ ] Add CSS rule `.sessions-refresh-stale`
- [ ] Existing `refresh_sessions_leaves_cache_untouched_when_disconnected` stays green
- [ ] `cargo test refresh_sessions sessions_section` green
- [ ] Commit: `feat(state): surface refresh failure via toast and stale chip`

**Checkpoint after B:** user kills daemon, clicks Refresh, verifies toast + stale chip.

## Slice C: Kill failure

- [ ] Red: `kill_session_id_blocking_returns_not_connected_when_disconnected` in `src/pty.rs::tests`
- [ ] Red: `kill_session_rpc_failure_does_not_remove_row_or_destroy_pane` in `src/state.rs`
- [ ] Update existing `session_kill_without_pane_mapping_drops_from_sessions_cache` test to match new contract (orphan + disconnected = row stays)
- [ ] Add `Command::KillAck` variant to `src/pty.rs`
- [ ] Worker handler awaits `client.kill_session` and replies
- [ ] Add `pub fn kill_session_id_blocking(&mut self, u64) -> io::Result<()>`
- [ ] Modify `mutate_kill_session_id` orphan branch: blocking call, retain row + push toast on Err
- [ ] Mapped-pane branch unchanged
- [ ] `cargo test session_kill kill_session_id` green
- [ ] Commit: `feat(pty): blocking kill_session_id variant`
- [ ] Commit: `feat(state): surface kill failure via toast`

**Checkpoint after C:** user kills daemon, clicks Kill on a session row, verifies row stays + toast.

## Slice D: Rename failure

- [ ] Red: `rename_commit_rpc_failure_keeps_dialog_open_with_error_string`
- [ ] Red: `rename_commit_rpc_failure_does_not_call_mutate_rename_pane`
- [ ] Red: `rename_dialog_renders_inline_error_when_present`
- [ ] Add `error: Option<String>` to `ConfirmDialog::RenameSession`
- [ ] Update all 17 construction sites (production + tests)
- [ ] Modify `dispatch("dialog.rename_commit")`: rebuild dialog with error, do not call `mutate_rename_pane`, no toast
- [ ] Modify input `on_change` in `build_rename_session_card` to clear `error` on type
- [ ] Modify `build_rename_session_card` to render inline error when `Some`
- [ ] Add CSS rule `.rename-session-error`
- [ ] `cargo test rename_commit rename_dialog` green
- [ ] Commit: `feat(state,ui): keep rename dialog open on rpc failure with inline error`

**Checkpoint after D:** user kills daemon, opens Rename, hits Enter, verifies inline error + original title preserved.

## Slice E: Integration + ship

- [ ] Create `tests/130_rpc_error_toasts.rs` with the integration sequence from plan section 2 slice E
- [ ] Run `/simplify` on every changed file
- [ ] `cargo test` green
- [ ] `cargo clippy --all-targets -- -D warnings` clean
- [ ] `cargo fmt --check` clean
- [ ] `cargo llvm-cov --html`: 100% line coverage on three dispatch arms + `push_error_toast`; full coverage on `toast.rs`
- [ ] Visual handoff to user using spec section 6.5 checklist (do NOT auto-screenshot)
- [ ] No changelog fragment (directory absent; verified)
- [ ] Push branch, open draft PR with body `fixes #130` and link to spec
- [ ] Mark PR ready once visual verification returns green
- [ ] Commit: `test: integration coverage for issue 130 error paths`

**Checkpoint after E:** PR open, all gates green, awaiting human review and merge.

## Out of scope (do not check off, do not implement)

- [ ] ~~Mapped-pane kill branch becomes blocking~~ (separate followup)
- [ ] ~~Info / warn / success toast levels~~ (out of scope)
- [ ] ~~Toast pauses on hover~~ (out of scope)
- [ ] ~~Notification history~~ (out of scope)
- [ ] ~~Click stale chip to retry~~ (chip is a marker, not a button)
