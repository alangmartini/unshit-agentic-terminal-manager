# PLAN: Surface daemon RPC errors (issue #130)

Spec: [`specs/130-rpc-error-toasts.md`](../specs/130-rpc-error-toasts.md)
Status: draft, awaiting review
Author: Alan Martini
Mode: read-only investigation done. No code changes yet.

## 0. Findings from the codebase

These constraints came from reading the actual files, not the spec.
They override the spec where they disagree.

1. **17 construction sites** for `ConfirmDialog::RenameSession` across
   `src/state.rs` and `src/ui/confirm_dialog.rs` (1 enum decl, 4 match
   arms, 12 build sites in code and tests). Adding the `error` field is
   a fan-out edit, not a single-file change. Plan accounts for this in
   slice D.
2. **Two fire-and-forget kill paths** exist in `src/pty.rs`:
   `destroy(pane_id)` at L311 (mapped-pane branch) and
   `kill_session_id(session_id)` at L325 (orphan branch). The issue
   names only the orphan branch. **In scope:** orphan branch becomes
   blocking. **Out of scope:** mapped-pane branch stays fire-and-forget;
   touching it is a separate followup.
3. **Blink subscription** runs every 500 ms (`src/bridge.rs:157`). At
   that cadence, 4 second auto-dismiss = 8 ticks. `default_lifetime: 8`
   in `ToastStore`.
4. **No `tests/` directory** at the worktree root. The spec's "tests/"
   integration test creates the directory. Cargo will pick it up
   automatically once a `*.rs` file appears there.
5. **No `changelog/unreleased/` directory.** The repo policy "every
   feat needs a changelog fragment" applies "if it exists". It does
   not. Skip the fragment, do not invent the directory.
6. **Worker handler** in `src/pty.rs:628` for `Command::Kill` calls
   `client.kill_session(session_id).await` and discards the result.
   Adding a reply requires either a new `Command::KillAck` variant or
   making `Command::Kill.reply` an `Option<SyncSender<...>>`. Plan
   chooses the new variant: simpler, leaves the fire-and-forget
   `Command::Kill` arm untouched for `destroy_all` and `destroy`.
7. **Root tree** in `src/main.rs:207-252` adds overlays in this order:
   ctx_menu, then confirm_dialog. Toasts go on top of confirm_dialog so
   they remain visible even when a modal is open.
8. **Existing test** `refresh_sessions_leaves_cache_untouched_when_disconnected`
   asserts only `state.sessions.len() == 1`. Adding `sessions_stale` as
   a new field does not break it.

## 1. Dependency graph

```
                +--------------------------+
                | T1 framework toast.rs    |  zero deps
                +-------------+------------+
                              |
                              v
+------------------+ +-----------------------+  +------------------+
| T3 pty shim     | | T2 state plumbing     |  | T5 ui overlay    |
| (independent)   | | (depends on T1 types) |  | (depends on T2)  |
+--------+--------+ +-----------+-----------+  +--------+---------+
         |                      |                       |
         |                      v                       |
         |          +-----------+-----------+           |
         |          | T8 bridge tick advance|           |
         |          | (T1 + T2 + render)    |           |
         |          +-----------+-----------+           |
         |                      |                       |
         +-----------+----------+-----------+-----------+
                     |                      |
                     v                      v
       +--------------------+   +-------------------------+
       | T4a refresh wiring |   | T4b kill wiring         |
       | (T2 + T7 chip)     |   | (T2 + T3)               |
       +---------+----------+   +-------------+-----------+
                 |                            |
                 +-------------+--------------+
                               |
                               v
                +------------------------------+
                | T4c rename wiring            |
                | (T2 + T6 inline error +      |
                |  17 construction sites)      |
                +-------------+----------------+
                              |
                              v
                +------------------------------+
                | T9 integration test + ship   |
                +------------------------------+
```

## 2. Vertical slices

Each slice ends with a green test suite, a clean clippy pass, a
formatted tree, and one atomic commit.

### Slice A: Toast plumbing visible end-to-end (foundation)

Three sub-tasks. Each sub-task is its own RGR cycle and its own commit.

#### A1. Framework toast module

**Touches:** `crates/unshit-framework/crates/unshit-core/src/toast.rs`
(new), `crates/unshit-framework/crates/unshit-core/src/lib.rs`
(`pub mod toast`).

**TDD steps:**
1. Red. Write `crates/unshit-core/src/toast.rs` with
   `#[cfg(test)] mod tests` exercising every spec test in 6.1:
   `push_under_cap_appends`, `push_at_cap_evicts_oldest`,
   `dismiss_removes_by_id_and_is_idempotent`,
   `advance_ticks_decrements_and_evicts_at_zero`,
   `advance_ticks_returns_dismissed_ids`,
   `iter_yields_in_push_order`. The module's pub items are stubs that
   make the file compile (`unimplemented!()` bodies). `cargo test
   -p unshit-core toast::tests::` fails.
2. Green. Implement `Toast`, `ToastKind` (single `Error` variant),
   `ToastStore { next_id, items, cap, default_lifetime }`. Methods:
   `with_capacity(cap, lifetime)`, `push(message) -> ToastId`,
   `dismiss(id) -> bool`, `advance_ticks(n) -> Vec<ToastId>`,
   `iter() -> impl Iterator<Item = &Toast>`, `len()`, `is_empty()`. No
   `Instant`, no real clock. `cargo test -p unshit-core toast::` is
   green.
3. Refactor. Run `/simplify` mentally: collapse trivial helpers, pick
   an idiomatic eviction (probably `Vec::remove(0)` is fine for cap=3),
   make sure no `unwrap()` slips in.
4. Re-export from `lib.rs`: `pub mod toast; pub use toast::{...};`.
   Decide: only the names listed in the spec
   (`Toast, ToastKind, ToastStore, ToastId`). Do NOT add to
   `prelude::*`; consumers import explicitly.

**Acceptance:**
- 6 unit tests in `toast.rs` pass.
- `cargo clippy -p unshit-core --all-targets -- -D warnings` clean.
- No new public items except the four named.

**Verification command:**
```bash
cargo test -p unshit-core toast
cargo clippy -p unshit-core --all-targets -- -D warnings
```

**Commit:** `feat(framework): toast notification primitive`

---

#### A2. App state plumbing (no UI yet)

**Touches:** `src/state.rs`.

**TDD steps:**
1. Red. Write three new tests in `src/state.rs::tests`:
   `push_error_toast_caps_at_three`,
   `dispatch_toast_dismiss_removes_toast`,
   `ui_snapshot_includes_toast_view_in_push_order`. Tests reference a
   `push_error_toast(state, "...")` helper, an `AppState.toasts` field,
   and a `UiSnapshot.toasts: Vec<ToastView>` field that do not yet
   exist. Compilation fails.
2. Green.
   - Import `ToastStore` and friends from `unshit::core::toast`.
   - Add `AppState.toasts: ToastStore` (initialized via
     `ToastStore::with_capacity(3, 8)` at every `seed_state` /
     `test_state` / construction site; grep first to find them all).
   - Add `AppState.sessions_stale: bool` initialized to `false`. Add
     to `UiSnapshot.sessions_stale` and copy in `ui_snapshot()`.
   - Define `ToastView { id, kind, message }` in `state.rs` (the
     spec puts it here, not in the framework).
   - Add `UiSnapshot.toasts: Vec<ToastView>` and copy in
     `ui_snapshot()` by mapping over the store's iter.
   - Implement `pub fn push_error_toast(state, msg: impl Into<String>)`.
   - Add dispatch arm: `other if other.starts_with("toast.dismiss:") =>
     parse u64, call state.toasts.dismiss(id), return true on hit`.
3. Refactor. Confirm no `clone()` on `Vec<ToastView>` per redraw is
   doing anything wasteful (it is the snapshot, so it's fine).

**Caveat:** every `AppState` constructor must initialize the new
fields. Grep first: `Grep AppState\s*\{ src/`. Likely sites:
`seed_state`, `test_state`, any `..Default::default()` patterns. Update
every one in the same commit so `cargo check` stays green.

**Acceptance:**
- 3 new tests pass.
- Existing tests in `state.rs` pass unchanged.
- Snapshot-shape changes do not break `src/ui/` builder tests
  (ToastView default is empty, sessions_stale default is false).

**Verification command:**
```bash
cargo check
cargo test --lib
cargo clippy --all-targets -- -D warnings
```

**Commit:** `feat(state): toast store, dismiss dispatch, sessions_stale flag`

---

#### A3. UI overlay, root wiring, CSS, and bridge tick

**Touches:** new `src/ui/toasts.rs`; `src/ui/mod.rs`;
`src/main.rs` (root tree); `assets/styles.css`; `src/bridge.rs`
(blink subscription).

**TDD steps:**
1. Red. Write builder tests in `src/ui/toasts.rs`:
   `toast_overlay_empty_returns_hidden_div`,
   `toast_overlay_renders_one_card_per_toast`,
   `toast_card_carries_role_status_and_aria_live`,
   `toast_card_click_dispatches_dismiss`. Builder is a stub.
2. Green.
   - `build_toast_overlay(snap, shared) -> ElementDef`. Empty list
     returns `Div.with_class("toast-overlay-hidden")`. Non-empty
     returns a fixed-position `Div.with_class("toast-overlay")` with
     `role="status"`, `aria-live="polite"`, `aria-atomic="false"`,
     containing one card per toast in push order. Each card has the
     toast's id baked into its element id (`toast-<id>`), the message
     as text, click handler dispatching `toast.dismiss:<id>`.
   - `src/ui/mod.rs`: `pub mod toasts;`.
   - `src/main.rs`: append `build_toast_overlay(snap, shared)` to the
     root tree after `build_confirm_dialog_overlay`.
   - `assets/styles.css`: add `.toast-overlay`, `.toast`,
     `.toast-error`, `.toast-overlay-hidden` rules. Position fixed
     bottom-right, above status bar (use a CSS variable matching the
     status bar height, or hard-code `bottom: 32px; right: 16px;`).
   - `src/bridge.rs`: in `cursor_blink_subscription`, after the
     existing cursor-visibility loop, call
     `state.toasts.advance_ticks(1)`. If the returned `Vec<ToastId>`
     is non-empty, the snapshot will already differ (the items are
     gone), so the existing redraw path picks it up.
3. Refactor. Strip any inline styles that should live in CSS. Confirm
   the overlay does not block clicks on the underlying UI when empty.

**Caveat:** `aria-*` and `role` may need `with_attribute(...)`. Check
the framework's `ElementDef` API before assuming it supports them.
If not, add them to the spec/plan as a framework fix (Framework-First
policy) before continuing.

**Acceptance:**
- 4 new builder tests pass.
- `cargo run` launches without panic, no toasts visible (because none
  pushed yet).
- Manual: in a scratch branch, temporarily `state.toasts.push(...)`
  in `seed_state` to confirm the overlay paints. Revert before
  commit.

**Verification command:**
```bash
cargo test toasts
cargo build
cargo run     # smoke check, no toast visible by design
```

**Commit:** `feat(ui): toast overlay, root wiring, blink-driven dismiss`

---

### Slice B: Refresh failure surfaces toast and stale chip

Smallest of the three real failure paths. First slice that produces a
user-visible behavior on a real RPC error.

**Touches:** `src/state.rs::refresh_sessions`,
`src/state.rs::dispatch("sessions.refresh")` (if any extra logic needed),
`src/ui/settings.rs::build_sessions_section`,
`assets/styles.css` (`.sessions-refresh-stale`).

**TDD steps:**
1. Red.
   - `refresh_sessions_failure_sets_stale_and_pushes_toast`: with
     a disconnected `pty_manager`, call `refresh_sessions(&mut state)`,
     assert `state.sessions_stale == true` and `state.toasts.len() == 1`.
   - `refresh_sessions_success_clears_stale`: pre-set
     `state.sessions_stale = true`, then ... we cannot easily stand up
     a real daemon in unit tests. Instead, factor `refresh_sessions`
     so the success path is reachable via a small in-memory
     `SessionInfo` builder. Easier path: in the unit test, set
     `state.sessions_stale = true`, call `refresh_sessions`, assert
     stale stays true (because still disconnected), then add an
     integration test in slice E that wires a fake daemon.
     Decision: in this slice, only test the failure path. Keep
     coverage of the success-path-clears-stale behavior for the
     integration test.
   - `sessions_section_renders_stale_chip_when_flag_set`: build the
     section with `snap.sessions_stale = true`, assert the rendered
     tree contains an element with class `sessions-refresh-stale`.
2. Green.
   - Modify `refresh_sessions`: on `Err`, set `state.sessions_stale =
     true` and call `push_error_toast(state, format!("refresh failed:
     {e}"))`. On `Ok`, set `state.sessions_stale = false`.
   - Modify `build_sessions_section`: when `state.sessions_stale`,
     append a small `Span.with_class("sessions-refresh-stale")` next
     to the refresh button. Text: `"stale"`.
   - Add the CSS rule.
3. Refactor.

**Acceptance:** A3 from spec.
- Existing
  `refresh_sessions_leaves_cache_untouched_when_disconnected` still
  green.
- Two new tests pass.
- Visual handoff at end of slice (user runs the daemon-killed flow).

**Verification command:**
```bash
cargo test refresh_sessions
cargo test sessions_section
cargo clippy --all-targets -- -D warnings
cargo run     # user verifies refresh path with daemon killed
```

**Commit:** `feat(state): surface refresh failure via toast and stale chip`

---

### Slice C: Kill failure leaves row and pushes toast

Adds a blocking variant in the pty shim (orphan branch only) and
rewires the dispatch.

**Touches:** `src/pty.rs` (Command enum, worker handler, new pub fn),
`src/state.rs::mutate_kill_session_id`.

**TDD steps:**
1. Red.
   - `kill_session_id_blocking_returns_not_connected_when_disconnected`
     in `src/pty.rs::tests`. Asserts the new shim method returns
     `Err(io::ErrorKind::NotConnected)` (or the existing `not_connected()`
     code) on `inner = None`.
   - `kill_session_rpc_failure_does_not_remove_row_or_destroy_pane`:
     seed `state.sessions` with a pure orphan (no pane mapping in
     `pty_manager.sessions`), dispatch `session.kill:<id>`, assert
     `state.sessions.len() == 1` (row preserved) and `state.toasts.len()
     == 1`.
   - `kill_session_rpc_success_removes_row` (regression): existing
     test from `state.rs:2792` already covers this in the
     unmapped-pane case but with fire-and-forget. We need to keep
     that working under the new blocking path. Decision: keep the
     existing test name; update the test to expect `state.sessions`
     to retain the row when disconnected, drop it when connected.
     The existing test uses a disconnected manager, so it currently
     drops the row. Under new behavior, with the blocking path, it
     should retain the row. Update the existing test's assertion
     accordingly. **Flag this as a behavior change** in the commit
     message.
2. Green.
   - In `src/pty.rs`: add `Command::KillAck { session_id, reply:
     SyncSender<io::Result<()>> }`. Worker handler awaits
     `client.kill_session(session_id).await`, maps to `io::Result<()>`,
     sends on reply. Keep `Command::Kill` for `destroy` and
     `destroy_all`.
   - Add `pub fn kill_session_id_blocking(&mut self, session_id: u64)
     -> io::Result<()>` mirroring `rename_session`: 2s timeout,
     `worker_gone()` mapping. Drop the local `inner.sessions.retain`
     line that the fire-and-forget version had; on success, retain
     the row removal in the caller (state.rs).
   - In `src/state.rs::mutate_kill_session_id`, orphan branch only:
     replace `state.pty_manager.kill_session_id(session_id)` with
     `match state.pty_manager.kill_session_id_blocking(session_id) {
       Ok(()) => state.sessions.retain(...),
       Err(e) => push_error_toast(state, format!("kill failed: {e}")),
     }`. Keep mapped-pane branch unchanged.
3. Refactor.

**Acceptance:** A2 from spec.

**Verification command:**
```bash
cargo test session_kill
cargo test kill_session_id
cargo clippy --all-targets -- -D warnings
cargo run     # user verifies kill path with daemon killed
```

**Commits (two atomic):**
- `feat(pty): blocking kill_session_id variant`
- `feat(state): surface kill failure via toast`

---

### Slice D: Rename failure keeps dialog with inline error

Highest fan-out slice (17 `RenameSession` construction sites).

**Touches:** `src/state.rs::ConfirmDialog::RenameSession` enum
variant, all 17 construction sites in `src/state.rs` and
`src/ui/confirm_dialog.rs` (production code and tests),
`src/ui/confirm_dialog.rs::build_rename_session_card`,
`src/state.rs::dispatch("dialog.rename_commit")`,
`assets/styles.css` (`.rename-session-error`).

**TDD steps:**
1. Red.
   - `rename_commit_rpc_failure_keeps_dialog_open_with_error_string`:
     seed a `RenameSession` dialog, dispatch `dialog.rename_commit`
     with disconnected pty, assert `state.confirm_dialog` is
     `Some(RenameSession { error: Some(_), .. })` and `state.toasts`
     is empty (per decision 1a from the spec, the error is inline
     only, not also a toast).
   - `rename_commit_rpc_failure_does_not_call_mutate_rename_pane`:
     same seed, assert the original pane title is unchanged after
     the failed dispatch.
   - `rename_commit_rpc_success_clears_dialog_and_renames_pane`:
     existing test (state.rs:2733). Update if the field shape
     change forces a tweak.
   - `rename_dialog_renders_inline_error_when_present`: builder test
     in `src/ui/confirm_dialog.rs`, asserts a node with class
     `rename-session-error` is present when error field is `Some`,
     absent when `None`.
2. Green.
   - Add `error: Option<String>` to `ConfirmDialog::RenameSession`.
   - Update all 17 construction sites: production code passes
     `error: None`, test code uses `..Default::default()` if there's
     a derive, otherwise explicit `error: None`.
   - In `dispatch("dialog.rename_commit")`: change the body so on
     `Err`, the dialog is rebuilt as `Some(RenameSession { pane_id,
     buffer, error: Some(format!("rename failed: {e}")) })` and
     `mutate_rename_pane` is **not** called. On `Ok`, behavior is
     unchanged (clear dialog, rename pane, save workspaces).
   - In the input `on_change` handler in
     `build_rename_session_card`: when the user types, clear
     `error` (set to `None`) so the error does not persist after the
     user starts editing.
   - In `build_rename_session_card`: if the dialog has
     `error: Some(s)`, render a small div with class
     `rename-session-error` and text `s` between the input and the
     buttons.
   - Add CSS rule.
3. Refactor.

**Caveat:** `ConfirmDialog` derives `PartialEq, Eq` (state.rs:65). The
new `error: Option<String>` field works with those derives.

**Acceptance:** A1 from spec.

**Verification command:**
```bash
cargo test rename_commit
cargo test rename_dialog
cargo clippy --all-targets -- -D warnings
cargo run     # user verifies rename path with daemon killed
```

**Commit:** `feat(state,ui): keep rename dialog open on rpc failure with inline error`

---

### Slice E: Integration test, simplify pass, ship

**Touches:** `tests/130_rpc_error_toasts.rs` (new), every changed file
(simplify pass).

**Steps:**
1. Create `tests/130_rpc_error_toasts.rs` with one test that:
   - Builds a `seed_state`-like AppState with a disconnected daemon.
   - Opens a rename dialog via dispatch.
   - Dispatches `dialog.rename_commit` and asserts
     dialog still open with error, toasts empty.
   - Dispatches `sessions.refresh`, asserts toasts has one entry
     and `sessions_stale == true`.
   - Dispatches `toast.dismiss:<id>`, asserts toasts empty,
     `sessions_stale` unchanged.
   - Dispatches `session.kill:<orphan-id>` for an orphan session in
     `state.sessions`, asserts the row is preserved and a new toast
     was pushed.
2. Run `/simplify` over each slice's changed files. Squash anything
   redundant. Tests stay green.
3. Run the full check matrix: `cargo test`, `cargo clippy
   --all-targets -- -D warnings`, `cargo fmt --check`.
4. `cargo llvm-cov --html`. Confirm 100% line coverage on the three
   new dispatch arms and `push_error_toast`. Confirm `toast.rs` is
   fully covered.
5. Visual handoff to user. Per
   `feedback_visual_loop_only_on_request.md`, do NOT auto-screenshot.
   Hand the user the test plan from the spec section 6.5.
6. Confirm no changelog fragment is needed (directory absent).
7. Push branch. Open PR with body referencing the spec and `fixes
   #130`. Default to draft until visual verification comes back
   green.

**Acceptance:** A6, A7, and the spec's section 6.6 coverage gate.

**Verification command:**
```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo llvm-cov --html
```

**Commits:**
- `test: integration coverage for issue 130 error paths`
- (Any pure-cleanup commits from `/simplify`)

---

## 3. Checkpoints

Hand-off to user (visual + decision):

- **After A3.** Smoke check: `cargo run`, app launches, no toast
  visible, no console errors. The mechanism exists with no live
  caller yet. Approve to continue.
- **After B.** First real failure path. Kill daemon, click Refresh in
  Sessions panel: toast appears, stale chip appears. Approve.
- **After C.** Kill row failure path. Kill daemon, click Kill on a
  session row: toast appears, row stays. Approve.
- **After D.** Rename failure path. Kill daemon, right-click tab,
  Rename, Enter: dialog stays, inline error visible, original title
  unchanged. Approve.
- **After E.** Coverage report attached. PR ready to mark non-draft.

If any checkpoint fails verification, stop and surface the issue
before continuing.

## 4. Risks and mitigations

| Risk | Mitigation |
|------|------------|
| Slice D fan-out (17 sites) drifts the build red across multiple commits | Update all sites in a single commit (`feat(state,ui): rename inline error`). Run `cargo check` continuously while editing. |
| `kill_session_id_blocking` behavior change drops a previously-passing test (`session_kill_without_pane_mapping_drops_from_sessions_cache`) | Acknowledge in slice C commit message; update the test to reflect the new contract (orphan + disconnected = row stays). |
| Framework `ElementDef` lacks `aria-*` / `role` attribute support | Verified: framework has no DOM, no AT bridge. Dropped from A5 scope; deferred to [`unshit-rust-framework#228`](https://github.com/alangmartini/unshit-rust-framework/issues/228). Visual toast still ships; announcement path picks up once #228 lands. |
| Bridge subscription advances toast lifetimes during a window where the user is staring at a toast and types into a slow shell, dropping the toast under their cursor | 4s lifetime is conservative; user can click to dismiss anyway. Out of scope to extend on hover. |
| Coverage gate fails on 100% line coverage of `push_error_toast` | The helper is one line plus a `push` call. If `cargo llvm-cov` shows a missed branch, write a one-liner test that calls it directly. |

## 5. Out of scope (do not silently expand)

- Mapped-pane kill branch (`destroy(pane_id)`).
- Toast levels other than `Error`.
- Toast on hover pause.
- Notification history or persistence.
- Daemon wire protocol changes beyond the new `Command::KillAck`
  shim variant.
- Rename inline error styled with full validation UX (no shake
  animation, no aria-invalid on the input). Plain class is enough.
- Sessions panel "click stale chip to retry" behavior. The chip is a
  marker, not a button.
