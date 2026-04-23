# SPEC: Framework CSS Spec Compliance

Status: draft, awaiting approval.
Owner: Alan (alan@typesense.org).
Scope: `crates/unshit-framework/` and resulting app cleanups.

## 1. Objective

Align the `unshit` framework layout model with standard browser CSS so `<Div>` and its siblings behave like their HTML equivalents. A developer who knows CSS should be able to reason about the framework with no extra rules. This removes the need for app-level workarounds (17 `flex-direction: column` overrides in `assets/styles.css`) and makes the framework predictable.

Concrete outcomes:

* **F1. Block-by-default display.** `<Div>` defaults to `display: block`, matching the HTML `<div>` default. Children of a block container lay out in block flow (vertically). Containers opt into flex or grid explicitly via `display: flex` or `display: grid`. Within an explicit flex container, `flex-direction` keeps the CSS spec default of `row`.
* **F2. Correct CSS Grid `1fr` expansion.** Tracks declared with `grid-template-columns` / `grid-template-rows` using `1fr` (and mixed `1fr` + fixed sizes) distribute the free space in the grid container as defined by CSS Grid Level 1. Nested grids behave the same.

## 2. Non-Goals

* Reimplementing the layout engine. Minimum viable fixes only.
* Changing the cell-metrics timing or PTY spawn lifecycle (those are documented behaviors, not bugs).
* Publishing the framework or pushing to `unshit-upstream`. Fixes stay in this repo for now.
* Introducing new display modes (`inline-block`, `inline-flex`, etc.) unless strictly required by the two fixes.
* Full CSS Grid Level 2 parity. We target the subset the app uses (`1fr`, fixed lengths, `auto`, basic `repeat()` if already present).

## 3. Commands

```bash
cargo build                                           # compile workspace
cargo check                                           # type check
cargo test                                            # full workspace tests
cargo test -p unshit-core                             # core layout + style unit tests
cargo test -p unshit-test                             # framework render/integration tests
cargo test -p unshit-test --test grid_layout          # grid-specific tests
cargo test -p unshit-test --test block_default        # new: block-default tests (F1)
cargo run                                             # launch terminal-manager app for visual verification
cargo clippy --workspace --all-targets                # must pass with no warnings
cargo fmt --check                                     # must pass
cargo llvm-cov                                        # coverage must not decrease
```

## 4. Project Structure

Primary files in scope (framework):

* `crates/unshit-framework/crates/unshit-core/src/style/types.rs`: `Display` enum and default values.
* `crates/unshit-framework/crates/unshit-core/src/style/cascade.rs`: initial computed values.
* `crates/unshit-framework/crates/unshit-core/src/style/parse.rs`: parser for `display`, `grid-template-*`, `1fr` tokens.
* `crates/unshit-framework/crates/unshit-core/src/tree.rs`: layout pass, where flex and grid sizing run.
* `crates/unshit-framework/crates/unshit-test/tests/grid_layout.rs`: extend with `1fr` cases.
* `crates/unshit-framework/crates/unshit-test/tests/block_default.rs`: new.

Primary files in scope (app):

* `assets/styles.css`: remove the 17 redundant `flex-direction: column` overrides once F1 lands.
* `src/ui/**` and `src/bridge.rs`: adjust any Rust call sites that relied on the old flex-by-default behavior.

Files explicitly out of scope:

* `src/pty/`, `src/terminal/`, `src/state.rs`: no changes expected.
* Anything under `crates/unshit-framework/crates/unshit-renderer/` beyond what the layout fixes force.

## 5. Code Style

* Rust 2021, `cargo fmt` clean, zero clippy warnings on `--all-targets`.
* Public API changes only where required by the spec; nothing else moves.
* No feature flags, no backwards-compat shims. The fix lands cleanly or not at all.
* No new comments beyond what the existing code uses. Keep them only where the "why" is non-obvious.
* Tests follow existing patterns in `crates/unshit-test/tests/` (render-based pixel assertions via the test harness).
* Changelog fragment under `changelog/unreleased/` per global git rules if that directory exists in the repo.

## 6. Testing Strategy

Red-Green-Refactor, per project CLAUDE.md. Every fix starts with a failing test.

**Framework level (mandatory):**

* `block_default.rs` (new):
  * A `<Div>` with no explicit `display` stacks two child `<Div>`s vertically in block flow.
  * A `<Div>` with `display: flex` and no `flex-direction` lays two children horizontally (row).
  * A `<Div>` with `display: flex; flex-direction: column` lays two children vertically.
* `grid_layout.rs` (extend):
  * Single-track `grid-template-columns: 1fr` gives the child the container width.
  * `grid-template-columns: 1fr 1fr` splits width in half.
  * `grid-template-columns: 100px 1fr` gives the remainder after the fixed track.
  * `grid-template-rows: 1fr 2fr` splits by weight.
  * A grid nested inside a grid expands `1fr` against the inner container's resolved size, not the outer.
* No existing test under `crates/unshit-test/tests/` may regress.

**App level (mandatory):**

* `cargo run` renders visually identical to current `main` after the workaround CSS is removed. The user verifies this visually (per memory: do not automate screenshots).
* Sidebar, workspace body, split panes, settings panel, activity items all render correctly without their `flex-direction: column` overrides.
* Every existing `cargo test` in the app stays green.

**Coverage:** `cargo llvm-cov` must not decrease for `unshit-core` or `unshit-test`.

## 7. Boundaries

**Always do:**

* Write a failing test before each fix (TDD).
* Keep framework changes in `crates/unshit-framework/`.
* After F1 lands, update `assets/styles.css` to strip the redundant overrides in the same PR (so the framework change and its app-side cleanup ship together).
* Update the project `CLAUDE.md` "Known Framework Limitations" section to mark fixed items as done.
* Run `cargo test`, `cargo clippy`, `cargo fmt --check`, and `cargo run` before each commit.
* `/simplify` each change before PR.

**Ask first:**

* Any public API change on `unshit-core` beyond the two defaults.
* Changes to the CSS parser grammar beyond what `1fr` and `display: block` require.
* Adding new layout modes or Rust types not listed in section 4.
* Touching files outside section 4 scope.

**Never do:**

* Introduce feature flags or compatibility shims.
* Break explicit `display: flex` or `display: grid` behavior (F1 only affects the unset default).
* Skip hooks (`--no-verify`) or bypass signing.
* Remove the eager PTY spawn or the cell-metrics timing logic.
* Push to `unshit-upstream`. Framework fixes stay in this repo for now.
* Add Claude attribution to commits.
* Use em dashes or stray dashes in prose.

## 8. Delivery Plan

Two independent PRs. Order is free; either can land first.

**PR A. F1 block-by-default.**

1. Failing test: `block_default.rs` cases above.
2. Flip `Display` default and initial cascade value.
3. Sweep app CSS: remove the 17 `flex-direction: column` overrides, keep only genuinely needed ones.
4. Sweep Rust call sites that depended on the old default.
5. `cargo run` visual check.
6. Update `CLAUDE.md`: remove or strike item 1 from "Known Framework Limitations".
7. Changelog fragment.

**PR B. F2 grid `1fr` expansion.**

1. Failing tests: extend `grid_layout.rs` with the five cases above.
2. Investigate `tree.rs` grid sizing: identify why `1fr` currently misbehaves.
3. Fix track expansion.
4. `cargo run` visual check (no regression; app currently avoids grid, so mainly test-driven).
5. Update `CLAUDE.md`: remove or strike item 2 from "Known Framework Limitations".
6. Changelog fragment.

## 9. Assumptions

These are the assumptions I am making. Correct me before I proceed.

1. The framework's existing `Display` enum and `flex-direction` code already exist (confirmed via grep in `style/types.rs` and `style/parse.rs`), so F1 is a default-value change plus tests, not a new layout mode.
2. The app has no intentional reliance on `<Div>` defaulting to flex-row. The 17 column overrides are evidence the default is actively unwanted.
3. Grid track sizing in `tree.rs` is the right layer to fix `1fr`. If the bug turns out to be in parse-time rather than layout-time, scope stays inside section 4 files.
4. Framework render tests in `crates/unshit-test/` are the correct acceptance bar. Visual app verification is by the user.
5. No need to push changes to `unshit-upstream` as part of this work.

## 10. Open Questions

None blocking. Raise during implementation if the grid investigation surfaces a deeper issue.
