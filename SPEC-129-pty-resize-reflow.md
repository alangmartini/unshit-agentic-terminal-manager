# SPEC: PTY resize reflow (grow lifts scrollback, shrink evicts to scrollback)

Status: approved
Owner: Alan
Tracking: https://github.com/alangmartini/unshit-agentic-terminal-manager/issues/129
Branch: feat/129-pty-resize-reflow

## 1. Objective

Splitting a pane and then unsplitting currently leaves the surviving pane in a bugged state: narrow-width prompts from the pre-split layout stay on the visible grid, a wide-width prompt appears somewhere mid-grid, and a multi-row blank gap separates the old content from the live prompt. Fix the reflow so that:

- Growing rows lifts scrollback lines into the newly available rows at the top. The cursor stays last-line-anchored (no blank gap before the live prompt).
- Shrinking rows evicts the top rows into scrollback instead of discarding them.
- The content is always bottom-anchored across a resize round-trip (split, then unsplit) the buffer matches what a well-behaved emulator (iTerm2, alacritty, xterm) produces.

### Target user

Anybody who splits and unsplits panes in terminal-manager. Today this is a visible rendering bug on every split / unsplit pair with scrollback present.

### Non-goals

- Column reflow (rewrapping wrapped lines when cols change). Current behavior still clips cells past the new col count. Separate project.
- Preserving wrap markers across a row resize.
- Adjusting shell behavior (bash still reemits its prompt on SIGWINCH; we do not try to suppress that).

## 2. High-level architecture

Two separate `Terminal` emulators live in this repo and both have the bug:

1. **Daemon-side authoritative emulator**, `crates/unshit-terminal-core/src/terminal.rs`. Owned by `Session` (`crates/unshit-ptyd/src/session/mod.rs:201`). Parses PTY bytes, maintains the canonical `Grid` + `Scrollback`, serves snapshots to the UI.
2. **UI-client mirror**, `src/terminal/mod.rs`. Receives `output` events from the daemon, re-parses them into a local `CellGrid` + `scrollback: VecDeque<Vec<Cell>>`, renders.

The framework primitive `CellGrid::resize` (`crates/unshit-framework/crates/unshit-core/src/cell_grid.rs:866`) is a pure top-left copy and does not know about scrollback. This is correct: scrollback is an app-level concept. The lift / evict logic lives at the `Terminal` layer on both sides. Framework is not changed by this issue.

Both emulators are fixed identically so the UI mirror and the daemon stay in sync during the resize round-trip (the UI runs a local resize before any replayed output arrives from the daemon).

## 3. Core features and acceptance criteria

### F1. Grow lifts scrollback, cursor stays anchored

> Terminal has N lines of scrollback and rows=R. Resize to rows=R+K. The K topmost visible rows are filled from the bottom of scrollback (newest first); scrollback shrinks by up to K. Cursor row stays on the same logical line (it moves down by the number of lines lifted).

Acceptance:
- If scrollback has >= K lines, the top K rows of the new grid equal the last K scrollback lines (in the same order they scrolled off), and scrollback length decreases by K.
- If scrollback has fewer than K lines (say M), the M lifted rows sit immediately above the existing content (rows `K-M..K`), the top `K-M` rows are blank, and scrollback is empty afterward. Cursor row always advances by K so it stays anchored to its distance-from-bottom regardless of how many rows were lifted.
- No visible blank gap between scrolled-back content and the live prompt when scrollback has content.

### F2. Shrink evicts top rows into scrollback

> Terminal has rows=R. Resize to rows=R-K. The top K rows are pushed into scrollback as the newest entries, and the cursor row moves up by K (clamped to 0 if K > cursor_row).

Acceptance:
- After shrink, the new grid rows 0..R-K equal the old grid rows K..R, including attributes.
- Scrollback length grows by K (clamped by `MAX_SCROLLBACK`; oldest entries evicted on overflow, same as the normal scroll path).
- Cursor row is `old_cursor_row.saturating_sub(K)`.
- If K > old cursor_row: the rows above the cursor are still evicted up to the cursor position only. Cursor row becomes 0; any row evicted from above the cursor is pushed to scrollback, rows at or after the cursor that no longer fit are dropped (or pushed to scrollback if blank tail trimming is preferred). We trim blank tail rows rather than pushing them into scrollback to avoid polluting history with padding.

### F3. Resize round-trip is stable

> Write N lines of output (N > R). Resize R -> R-K. Resize R-K -> R. The resulting grid equals the state that would exist if we had always rendered at R.

Acceptance:
- After the round-trip the live grid contents + scrollback tail reconstruct the pre-resize sequence.
- No duplicated, missing, or reordered lines.

### F4. Column resize preserves row behavior

> Columns change without row change: same behavior as today (cells past new cols are clipped, cursor col is clamped). No scrollback side effects.

Acceptance:
- `Terminal::resize(R, new_cols)` with `new_cols != cols` and same R does not modify scrollback.

## 4. Project structure

Files to touch (small, focused):

- `crates/unshit-terminal-core/src/grid.rs`
  - Add `shrink_rows_from_top(&mut self, n: usize) -> Vec<Vec<Cell>>` that removes the top `n` rows and returns them as owned vecs. The remaining rows shift up; the bottom gains `n` blank rows so the row count is preserved (the caller's subsequent `resize` truncates the blank tail to the final row count).
  - Add `grow_rows_at_top(&mut self, n: usize, lifted: impl IntoIterator<Item = Vec<Cell>>)` that grows the grid by `n` rows added at the top, painted with the lifted rows. Lifted rows shorter than `cols` are padded; longer ones are clipped. If fewer than `n` rows are lifted, they sit at the bottom of the new top section so they're adjacent to the existing content.
- `crates/unshit-terminal-core/src/terminal.rs::resize`
  - On grow: lift up to `K = new_rows - old_rows` lines from the tail of `scrollback` and call `grid.grow_rows_at_top`. Advance cursor_row by the full K so the bottom of the grid stays anchored.
  - On shrink: `K = old_rows - new_rows`. Evict up to `min(K, cursor_row)` top rows into scrollback (so the live prompt row is never pushed). Decrement cursor_row by the evicted count. Any remaining shrink trims the blank tail below the cursor via the subsequent `grid.resize`.
  - Column-only resize delegates to `grid.resize` with no scrollback side effects.
- `crates/unshit-terminal-core/src/scrollback.rs`
  - Add `pop_back_n(&mut self, n: usize) -> Vec<Vec<Cell>>` that pops up to `n` newest lines off the back, returned in oldest-first order so callers can place them top-to-bottom in a grid.
- `src/terminal/mod.rs::resize`
  - Mirror the daemon logic. Reuse the existing `scrollback: VecDeque<Vec<Cell>>` and `MAX_SCROLLBACK` constant.
  - On grow: lift from the back of `scrollback` into new top rows of `self.grid`. Advance `self.cursor_row`.
  - On shrink: capture top rows, push_back to `scrollback`, pop_front if overflow, adjust `self.cursor_row`.
  - Reuse `CellGrid::resize` for the resize-then-write-into path; we shift first, then call resize, then paint lifted cells.

Files NOT touched:

- `crates/unshit-framework/crates/unshit-core/src/cell_grid.rs` — no framework change. Scrollback is an app concern.
- `crates/unshit-ptyd/src/session/mod.rs` — `Session::resize` stays as-is; it already calls `term.resize` which gets the new logic for free.
- `src/pty.rs`, `src/bridge.rs`, `src/state.rs` — no change. The resize call chain is unchanged.

## 5. Code style

- Rust, 2024 edition (follow the rest of the repo).
- No framework changes unless a bug in the framework is discovered during implementation. If one is, fix it upstream per the Framework-First Development policy and note it in the PR.
- New helpers go on `Grid` / `Scrollback` rather than inlining vec manipulations in `Terminal::resize`.
- Keep `Terminal::resize` short and obvious: compute delta, call grid helper, push/pop scrollback, clamp cursor. No surprise comments, no WHAT comments. Docstring states the behavior (bottom-anchored reflow), not the call chain.
- No feature flag. This is a bug fix; the old behavior has no users who want it.

## 6. Testing strategy

### Unit tests (mandatory for every new helper and branch)

**`crates/unshit-terminal-core/src/grid.rs`**
- `shrink_rows_from_top_returns_top_rows_and_shifts_remainder`
- `shrink_rows_from_top_handles_n_greater_than_rows`
- `grow_rows_with_lift_prepends_lifted_rows`
- `grow_rows_with_lift_pads_with_blank_when_iterator_shorter`

**`crates/unshit-terminal-core/src/terminal.rs`**
- `resize_grow_lifts_scrollback_into_new_top_rows` (regression, comment references issue #129)
- `resize_grow_with_empty_scrollback_fills_bottom_with_blank_rows_and_keeps_cursor` (cursor doesn't drift)
- `resize_shrink_pushes_top_rows_to_scrollback_and_decreases_cursor_row`
- `resize_shrink_respects_max_scrollback` (drops oldest on overflow)
- `resize_round_trip_is_stable` (write N > R lines, shrink, grow, assert grid + scrollback tail)
- `resize_column_only_does_not_touch_scrollback`
- `resize_to_zero_rows_does_not_panic`

**`src/terminal/mod.rs`**
- Mirror of each of the daemon-side tests above. The UI terminal uses `CellGrid` from the framework so helpers differ, but behavior must match.

### Integration tests

- `tests/` directory: add a regression test that constructs a `Terminal` (PTY-free), writes a scripted sequence that mimics "prompt, command, output, prompt", resizes down, writes more, resizes back up, and asserts no blank gap and no interleaved narrow prompts. Comment references issue #129.

### Manual verification (required per CLAUDE.md Agent/Worktree Guidelines §1)

- `cargo run`. Open the app. Run commands so scrollback accumulates. `Ctrl+Shift+D` to split. Run a command in the narrow pane. `Ctrl+W` on the sibling to unsplit. Verify the surviving pane has no blank gap, no interleaved narrow-width prompts, and the live prompt is at the bottom.
- Repeat with drag-split.
- Repeat across a daemon detach/attach (close app, reopen): reattach snapshot must reflect the bottom-anchored grid too (the daemon owns the source of truth).

### Coverage

- `cargo llvm-cov` before and after. New code paths must be covered; coverage must not decrease.

## 7. Boundaries

### Always do

- Add a regression test referencing issue #129 in its comment.
- Fix the same logic in both emulators (daemon and UI mirror) in a single PR.
- Follow Red-Green-Refactor: failing regression test first, minimum fix, then `/simplify`.
- Run `cargo test`, `cargo clippy`, `cargo fmt --check` before committing.
- Verify visually via `cargo run` after the code passes (tests passing does not prove the rendering is correct — per CLAUDE.md guidance).

### Ask first

- Before introducing a new public helper on `CellGrid` in the framework (we said no framework change; if the implementation suggests one is needed, confirm).
- Before changing the `output` event protocol between daemon and UI.
- Before adding column reflow. Column reflow is explicitly out of scope for this issue.
- Before changing `MAX_SCROLLBACK` or the scrollback data structure.

### Never do

- Do not delete scrollback rows silently during shrink. They either go into scrollback or (in the `k > cursor_row` case) get trimmed from the blank tail only.
- Do not touch `Session::resize` signatures or the PTY resize call.
- Do not introduce a feature flag for the old behavior.
- Do not bypass `/simplify` on the PR.
- Do not skip hooks on commit, per the repo git rules.

## 8. Resolved decisions

1. **Shrink with `k > cursor_row`**: trim blank tail rows from below first, then evict from the top. Never silently drop real content above the cursor.
2. **Column normalization on lift**: lifted scrollback rows are clipped to new cols and padded with blank cells if the stored row is shorter. Matches how `display_grid` already tolerates uneven scrollback rows.
3. **Flat scrollback**: scrollback stays one-row-per-vec. No logical-line tracking. Soft-wrap aware scrollback is a column-reflow concern and is deferred to a separate issue.
4. **Both emulators fixed in one PR**: daemon-side (`crates/unshit-terminal-core`) and UI mirror (`src/terminal/mod.rs`) ship together so the round-trip stays consistent during a resize.
