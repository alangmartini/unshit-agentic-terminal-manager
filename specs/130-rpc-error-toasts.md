# SPEC: Surface daemon RPC errors in session rename / kill / refresh

Tracks: [#130](https://github.com/alangmartini/unshit-agentic-terminal-manager/issues/130)
Branch base: `feat/rust-terminal-manager`
Owner: Alan Martini
Status: draft, awaiting approval

## 1. Objective

Three UI paths that talk to `unshit-ptyd` swallow daemon errors and proceed
as if the call succeeded:

1. `dialog.rename_commit` (`src/state.rs:1647`) logs and still updates the
   local pane title, so the Sessions panel reverts on next refresh.
2. `mutate_kill_session_id` (`src/state.rs:1305`) sends a fire-and-forget
   `Command::Kill` and optimistically removes the row from
   `state.sessions` even when the daemon never acked.
3. `refresh_sessions` (`src/state.rs:1281`) logs a warning and leaves the
   stale cache on screen with no visible cue that the refresh failed.

The user sees no signal in any of those failure paths. We will introduce a
minimal toast notification primitive in `unshit-framework` and wire all
three dispatches through it, plus add an inline error inside the rename
dialog and a stale marker on the Sessions panel.

### Target users

The terminal manager's end users (developers running long-lived shells
under the daemon). The framework primitive also exists for any future
`unshit-framework` consumer that needs ephemeral notifications.

### Non-goals

- No general notification center, no history, no persistence.
- No info / warn / success levels in this slice. Error-only.
- No daemon-side wire changes beyond making `Command::Kill` ack-capable in
  the shim layer.
- No replacement of `confirm_dialog`. Toasts are non-blocking and stack on
  top of any modal that happens to be open.
- No accessibility audit beyond the polite live-region attributes
  specified in section 5.

## 2. Acceptance criteria

Mirrors the issue's checklist plus the decisions taken during
clarification.

- [ ] **A1.** When `rename_session` returns `Err`, the rename dialog stays
      open, an inline error string is rendered under the input, and the
      local pane title is **not** updated.
- [ ] **A2.** When `kill_session_id` cannot get an ack (disconnected
      daemon or 2 second timeout), the row is **not** removed from
      `state.sessions`, no local pane is destroyed, and a toast is pushed.
- [ ] **A3.** When `list_sessions` returns `Err`, the cached rows stay on
      screen, a "stale" marker appears next to the Refresh button, and a
      toast is pushed. Pressing Refresh again clears the stale marker on
      success.
- [ ] **A4.** Toasts auto-dismiss after roughly 4 seconds, stack upward
      from the bottom-right corner above the status bar, cap at 3
      visible, and dismiss on click.
- [ ] **A5.** The toast container carries `role="status"` and
      `aria-live="polite"`. Auto-dismissed toasts do not re-announce.
- [ ] **A6.** Unit tests cover the error path for each of the three
      dispatches, plus a builder-level render test for the toast overlay
      and the stale marker.
- [ ] **A7.** `cargo test`, `cargo clippy`, `cargo fmt --check` all pass.
      Coverage on `state.rs` for the three dispatch arms does not
      regress.

## 3. Commands

Standard repo commands, unchanged. Listed here so an implementer can copy
them verbatim.

```bash
cargo build
cargo run
cargo check
cargo test
cargo test --lib
cargo test --test '*'
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo llvm-cov --html
```

Verification that the slice is done end to end:

```bash
cargo test -- --nocapture toast      # toast unit and render tests
cargo test rename_commit             # rename failure path
cargo test refresh_sessions          # refresh failure path
cargo test session_kill              # kill failure path
cargo run                            # visual sanity check
```

## 4. Project structure

### 4.1 New files

```
crates/unshit-framework/crates/unshit-core/src/toast.rs
    Toast model: id, message, kind (error-only for now), remaining_ticks.
    ToastStore: push, dismiss(id), advance_ticks(n) -> Vec<Id> dismissed,
    iter, len. No real clock; tick-based for determinism in tests.

crates/unshit-framework/crates/unshit-core/src/lib.rs
    Add `pub mod toast;` and re-export `Toast`, `ToastKind`, `ToastStore`,
    `ToastId`.

src/ui/toasts.rs
    `build_toast_overlay(snap: &UiSnapshot, shared: &SharedState) -> ElementDef`.
    Renders the stacked toast list at fixed bottom-right. Each toast is
    a clickable card that dispatches `toast.dismiss:<id>`. Returns a
    hidden div when the list is empty.
```

### 4.2 Modified files

```
src/state.rs
    + AppState.toasts: ToastStore
    + AppState.sessions_stale: bool
    + UiSnapshot.toasts: Vec<ToastView>
    + UiSnapshot.sessions_stale: bool
    + push_error_toast(state, msg)
    Modify:
      - refresh_sessions: on Err, set sessions_stale = true and push toast.
                          on Ok, set sessions_stale = false.
      - mutate_kill_session_id: switch to blocking shim call; on Err leave
                                state.sessions untouched and push toast.
      - dispatch("dialog.rename_commit"): on Err, keep the dialog (do not
                                          take()), set buffer error field,
                                          do not call mutate_rename_pane.
      - dispatch("toast.dismiss:<id>"): new arm.
      - dispatch("sessions.refresh"): clears sessions_stale on success path
                                      via refresh_sessions.

src/state.rs (ConfirmDialog::RenameSession)
    Add `error: Option<String>` field. Cleared when the user types again.

src/pty.rs
    + kill_session_id_blocking(&mut self, session_id: u64) -> io::Result<()>
      Mirrors rename_session: send Command::Kill with reply_tx, recv with
      2 second timeout, map errors via worker_gone(). Keep
      `kill_session_id` as the fire-and-forget variant for destroy_all.

crates/unshit-ptyd/.../command.rs (or equivalent)
    Add a `reply: oneshot<io::Result<()>>` field to Command::Kill or a new
    Command::KillAck variant if the existing handler cannot be retrofitted
    without breaking destroy_all.

src/ui/confirm_dialog.rs
    build_rename_session_card: render the new `error` field as a small
    error string under the input when populated. Add corresponding CSS
    hook class `rename-session-error`.

src/ui/settings.rs (or wherever the Sessions panel lives)
    Render a "stale" tag next to the Refresh button when
    snap.sessions_stale is true. Tag clears on a successful refresh.

src/ui/mod.rs
    Wire build_toast_overlay into the root tree, on top of confirm_dialog.

assets/styles.css
    .toast-overlay { position: fixed; bottom: <statusbar height + gap>; right: ...; }
    .toast { padding, border, color, animation. }
    .toast-error { red accent. }
    .rename-session-error { error text under the rename input. }
    .sessions-refresh-stale { warning chip next to Refresh. }

src/bridge.rs
    Tick the toast store from the existing blink subscription path so
    auto-dismiss runs without a separate timer. Each blink tick advances
    toast lifetimes by the blink interval and triggers a redraw if any
    toast was dismissed.
```

### 4.3 Data shapes

```rust
// unshit-framework
pub type ToastId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind { Error }

#[derive(Clone, Debug)]
pub struct Toast {
    pub id: ToastId,
    pub kind: ToastKind,
    pub message: String,
    pub remaining_ticks: u32,
}

pub struct ToastStore {
    next_id: ToastId,
    items: Vec<Toast>,
    cap: usize,            // 3
    default_lifetime: u32, // ticks for ~4s
}

// app: UiSnapshot.toasts
#[derive(Clone, Debug)]
pub struct ToastView {
    pub id: ToastId,
    pub kind: ToastKind,
    pub message: String,
}
```

## 5. Code style

Same as the rest of the project. A short list of project-specific
constraints that apply here:

- Avoid em-dashes and en-dashes in comments and strings (per repo
  policy). Use commas, colons, or parentheses.
- No `unwrap()` or `expect()` in non-test code on RPC responses.
- All new public APIs in `unshit-core` get a one-paragraph rustdoc and at
  least one `# Examples` block where it does not require live state.
- Keep the framework module pure. No `Instant::now`, no spawned threads,
  no logging that ties to the app. Drive lifetime via tick counts so
  tests stay deterministic.
- App-side error helpers go behind `state::push_error_toast`. The three
  dispatches do not format the user-facing string themselves; they call
  the helper with a context tag (e.g. `"rename"`) and the underlying
  `io::Error` Display.
- Strings rendered to the user follow existing tone: lowercase, terse.
  Examples: `"rename failed: not connected"`, `"kill failed: timeout"`,
  `"refresh failed: not connected"`.
- Accessibility attributes: the toast container gets `role="status"`,
  `aria-live="polite"`, `aria-atomic="false"`. Auto-dismiss does not
  trigger a re-announce; the toast is removed from the DOM in place.
- CSS lives in `assets/styles.css`. No inline styles in the toast
  builder except positioning that depends on framework Dimension types
  (matches the pattern in `confirm_dialog.rs`).

## 6. Testing strategy

### 6.1 Unit tests in `crates/unshit-core/src/toast.rs`

- `push_under_cap_appends`
- `push_at_cap_evicts_oldest`
- `dismiss_removes_by_id_and_is_idempotent`
- `advance_ticks_decrements_and_evicts_at_zero`
- `advance_ticks_returns_dismissed_ids`
- `iter_yields_in_push_order`

### 6.2 Unit tests in `src/state.rs`

- `rename_commit_rpc_failure_keeps_dialog_open_with_error_string`
- `rename_commit_rpc_failure_does_not_call_mutate_rename_pane`
- `rename_commit_rpc_success_clears_dialog_and_renames_pane`
  (regression)
- `kill_session_rpc_failure_does_not_remove_row_or_destroy_pane`
- `kill_session_rpc_failure_pushes_one_error_toast`
- `kill_session_rpc_success_removes_row` (regression)
- `refresh_sessions_failure_sets_stale_and_pushes_toast`
- `refresh_sessions_success_clears_stale`
- `dispatch_toast_dismiss_removes_toast`
- `push_error_toast_caps_at_three`

The `pty_manager` already supports a not-connected mode (`inner: None`),
so the existing
`refresh_sessions_leaves_cache_untouched_when_disconnected` test pattern
extends naturally. For `kill_session`, the new
`kill_session_id_blocking` follows the `list_sessions` pattern and
returns `Err(not_connected())` when `inner` is `None`. That keeps the
test surface honest and avoids a test-only branch in production code.

### 6.3 UI builder render tests

- `toast_overlay_empty_returns_hidden_div`
- `toast_overlay_renders_one_card_per_toast`
- `toast_card_carries_role_status_and_aria_live`
- `rename_dialog_renders_inline_error_when_present`
- `sessions_panel_renders_stale_chip_when_flag_set`

These are pure tree-shape assertions on `ElementDef`, in line with the
existing tests in `src/ui/`.

### 6.4 Integration sanity

A single integration test in `tests/`: drive a `dispatch` sequence that
opens the rename dialog, simulates an RPC error path via a disconnected
`DaemonPty`, asserts the dialog is still open and a toast is present,
then dispatches `toast.dismiss:<id>` and asserts the store is empty.

### 6.5 Visual verification

Per `CLAUDE.md` the user does visual verification by hand. After
implementation:

1. `cargo run`.
2. Disable the daemon socket by killing `unshit-ptyd` while the UI is up.
3. Open Sessions panel, click Refresh: toast plus stale chip.
4. Right-click a tab, Rename, type something, Enter: dialog stays,
   inline error visible, original pane title unchanged.
5. Open Sessions panel, click Kill on a row: row stays, toast appears.

The above is for the user; the agent does not run the visual loop unless
explicitly asked (per `feedback_visual_loop_only_on_request.md`).

### 6.6 Coverage gate

`cargo llvm-cov --html` must show 100% line coverage on the three new
dispatch arms and the new `push_error_toast` helper. The framework
`toast.rs` module must be fully covered by its own unit tests.

## 7. Boundaries

### Always do

- Fix in the framework first when the change is generic UI. The toast
  primitive lives in `crates/unshit-framework/`.
- Write the failing test first (Red), then the minimum code (Green),
  then simplify (Refactor). The repo enforces TDD.
- Add a regression test referencing issue 130 in a comment for each of
  the three error paths.
- Use atomic conventional commits: `feat(framework): toast primitive`,
  `feat(state): error toast on rpc failure`, `feat(ui): inline rename
  error and sessions stale chip`, `test:` for any pure-test commit.
- Run `cargo clippy` and `cargo fmt --check` before each commit.
- Add a changelog fragment in `changelog/unreleased/` if that directory
  exists at implementation time (check before writing).
- Open a draft PR early, push WIP commits, never end a session with
  uncommitted code.

### Ask first

- Any change to the daemon wire protocol beyond making `Command::Kill`
  ack-capable. The shim layer should be enough; if it is not, surface
  the question.
- Any change that adds a third party crate (animation library, time
  utility, etc).
- Renaming or moving the existing `refresh_sessions`,
  `mutate_kill_session_id`, or `dialog.rename_commit` arms. Those are
  referenced in tests and other code paths.
- Switching auto-dismiss away from tick-based timing toward a real
  clock.

### Never do

- Do not introduce em-dashes or en-dashes in code, comments, docs, or
  user-visible strings.
- Do not add Claude as a co-author or co-committer on commits.
- Do not skip git hooks (`--no-verify`) or amend a hook-failed commit.
  Fix the underlying issue and create a new commit.
- Do not auto-screenshot, move the cursor, or steal foreground focus
  for visual verification. The user runs the visual loop manually.
- Do not regress the existing
  `refresh_sessions_leaves_cache_untouched_when_disconnected` test or
  the rename dialog open/close behavior.
- Do not remove the eager PTY spawn at startup as a side effect of any
  refactor here.
- Do not log secrets, customer names, or internal cluster ids in toast
  messages or errors. Errors come from `io::Error` Display only.
- Do not block the UI thread on RPC: `kill_session_id_blocking` uses
  the same 2 second timeout pattern as `list_sessions` /
  `rename_session`.

## 8. Implementation order (preview, not part of the spec contract)

1. Framework: `toast.rs` module plus tests plus lib re-export.
2. App state: `toasts` and `sessions_stale` fields, `push_error_toast`,
   `toast.dismiss:<id>` dispatch arm, snapshot fields. No UI yet.
3. Pty shim: `kill_session_id_blocking`, plumb a reply through
   `Command::Kill`.
4. Wire the three dispatches to push toasts on `Err`.
5. UI: `toasts.rs` builder, root tree wiring, CSS.
6. Rename dialog inline error: `ConfirmDialog::RenameSession.error`
   field, builder change, dispatch behavior on Err.
7. Sessions panel stale chip: render flag, clear on success.
8. Bridge: tick toast store from blink subscription.
9. Integration test, visual handoff to user, changelog fragment, PR.
