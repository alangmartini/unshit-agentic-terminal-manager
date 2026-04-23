# Terminal Manager

Rust-based terminal manager with PTY backend and web frontend, built on the `unshit` framework.

## Build & Run

```bash
cargo build          # compile
cargo run            # run the app
cargo check          # type-check without building
cargo test           # run all tests
cargo test --lib     # unit tests only
cargo test --test '*' # integration tests only
cargo llvm-cov       # coverage summary
cargo llvm-cov --html # coverage report in target/llvm-cov/html/
```

## Architecture

- `src/` - Rust backend: PTY management, terminal emulation, state, UI bridge
- `src/pty/` - Pseudo-terminal handling via `portable-pty`
- `src/terminal/` - Terminal emulation via `vte`
- `src/bridge.rs` - Frontend-backend bridge (PTY subscriptions, cursor blink, resize polling)
- `src/state.rs` - Application state, PTY dimension helpers, cell metrics
- `src/ui/` - UI components (split panes, settings, keyboard nav)
- `assets/styles.css` - All CSS styling
- `crates/unshit-framework/` - The unshit framework (git subtree from `unshit-rust-framework` repo)

## Framework-First Development

**We own the `unshit` framework precisely so we can fix its bugs and limitations upstream.** That is the whole point of maintaining it inside this repo as a subtree. When you hit a framework bug, missing capability, or awkward default, the default response is to **fix it in `crates/unshit-framework/`**, not to work around it in the app.

Rules:

1. **Fix the framework, not the symptom.** If a change would only be needed because the framework is wrong, the framework is what needs to change. App CSS/code should not exist solely to compensate for framework defaults that are wrong.
2. **App-level fixes are reserved for app-specific concerns.** Examples: PTY spawn lifecycle, terminal emulation wiring, app-specific layout choices. Anything that belongs to a general-purpose UI toolkit goes in the framework.
3. **If a framework fix is out of scope right now**, document it as a known limitation (see below) with a clear note that it is a workaround, not the intended state.

## Known Framework Limitations

Issues in `crates/unshit-framework/` that currently require care in the app. Prefer fixing these upstream when touched. The workarounds listed are temporary.

1. **CSS grid `1fr` expansion is unreliable.** Flexbox (`display: flex` + `flex: 1`) currently works where `display: grid` with `grid-template-*` does not. **Fix upstream:** repair grid track expansion in the framework layout engine.

2. **Cell metrics timing (not a bug, behavior to know).** The renderer publishes `cell_w`/`cell_h` via `CellGrid::publish_cell_metrics()` during the render pass. The `on_resize` handler fires during the layout pass BEFORE the renderer, so `global_cell_w()` is 0.0 on the first frame. The `on_cell_metrics` callback and blink subscription handle the correction.

3. **PTY must be spawned eagerly (app-level concern, keep as is).** Do NOT defer PTY spawning until cell metrics are available. This creates a deadlock: no terminal, no CellGrid rendered, no metrics published, PTY never spawns. Always spawn at 80x24, then correct dimensions once metrics arrive.

## Agent/Worktree Guidelines

The unshit framework lives inside this repo as a git subtree at `crates/unshit-framework/`. Agents in worktrees can freely modify both app code and framework code without path issues, and per the **Framework-First Development** policy above, they are **expected** to fix framework issues in `crates/unshit-framework/` rather than patch around them in app code.

1. **Run `cargo run` after merging agent work.** Tests passing does NOT mean the app works correctly. Visual regressions (layout, CSS) are not caught by unit tests. Always launch the app and verify visually.

2. **Merge agents one at a time, not all at once.** Parallel agent branches that touch overlapping files create cascading merge conflicts. Merge and verify each one sequentially.

3. **Check for missing callbacks after conflict resolution.** `AppConfig` fields like `on_close`, `on_scale_factor`, `on_cell_metrics` can be silently dropped during conflict resolution. Verify all callbacks are present after merging.

4. **Do not remove eager PTY spawn.** The initial PTY spawn in `main.rs` is load-bearing. Without it, the terminal never renders.

## Syncing with upstream unshit

The framework was imported via `git subtree` from `unshit-rust-framework`. The remote `unshit-upstream` is configured.

```bash
# Pull latest changes from the standalone unshit repo
git subtree pull --prefix=crates/unshit-framework unshit-upstream master --squash

# Push framework changes back to the standalone repo
git subtree push --prefix=crates/unshit-framework unshit-upstream <branch-name>
```

## Testing Requirements

Every feature and bug fix MUST have proper test coverage. No exceptions.

### Unit Tests
- Every module must have a `#[cfg(test)] mod tests` block
- Test all public functions, edge cases, and error paths
- Aim for full code coverage on business logic

### Integration Tests
- Place in `tests/` directory
- Test cross-module interactions (PTY + terminal emulation, state + bridge)
- Test the full data flow from input to output

### E2E Tests
- Test the complete application behavior
- Verify frontend-backend communication
- Test split pane operations, keyboard navigation, settings

### Regression Tests
- Every bug fix MUST include a regression test that reproduces the original bug
- The test must fail without the fix and pass with it
- Include a comment referencing the issue number

## Development Workflow: TDD (mandatory)

Follow strict Test-Driven Development for all feature work:

1. **Red** - Write a failing test that defines the desired behavior
2. **Green** - Write the minimum code to make the test pass
3. **Refactor** - Clean up the code. Run `/simplify` after each green phase.
4. Repeat until the feature is complete

For bug fixes, follow Red-Green-Refactor:
1. **Red** - Write a failing test that reproduces the bug
2. **Green** - Fix with the simplest change possible
3. **Refactor** - Clean up. Run `/simplify`.

## Code Quality Gates

### Before Every Commit
- `cargo test` must pass
- `cargo clippy` must pass with no warnings
- `cargo fmt --check` must pass
- `cargo llvm-cov` to verify coverage (requires `cargo install cargo-llvm-cov`)

### Before Every PR
- Run `/simplify` on all changed code
- All tests pass
- No clippy warnings
- Code is formatted

### PR Review Requirements
- Every PR goes through `/simplify` before merge
- PRs are reviewed by the Claude Code GitHub Action for quality, correctness, and security
- Test coverage must not decrease
