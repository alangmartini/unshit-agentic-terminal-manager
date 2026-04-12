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
- Framework dependency: `../unshit-rust-framework/` (relative to repo root, NOT deeper)

## Framework Pitfalls (unshit-rust-framework)

Critical things to know about the unshit framework:

1. **Default display is flex-row.** Every `<Div>` defaults to `display: flex; flex-direction: row`. Any container that stacks children vertically MUST have `flex-direction: column` in CSS. This includes `.workspace-body`, `.sidebar-scroll`, `.sidebar-footer`, `.activity-item`, etc.

2. **CSS grid may not expand children correctly.** Prefer flexbox (`display: flex` + `flex: 1`) over `display: grid` with `grid-template-*` for layout. Grid support exists but `1fr` expansion has issues.

3. **Cell metrics timing.** The renderer publishes `cell_w`/`cell_h` via `CellGrid::publish_cell_metrics()` during the render pass. The `on_resize` handler fires during the layout pass BEFORE the renderer, so `global_cell_w()` is 0.0 on the first frame. The `on_cell_metrics` callback and blink subscription handle the correction.

4. **PTY must be spawned eagerly.** Do NOT defer PTY spawning until cell metrics are available. This creates a deadlock: no terminal -> no CellGrid rendered -> no metrics published -> PTY never spawns. Always spawn at 80x24, then correct dimensions once metrics arrive.

5. **Cargo.toml path dependency.** The unshit path MUST be `../unshit-rust-framework/crates/unshit`. Agents working in worktrees will see a different relative path. Always verify Cargo.toml after agent work.

## Agent/Worktree Guidelines

When spawning agents in isolated worktrees:

1. **Always verify `Cargo.toml` path dependencies after agent work.** Worktrees change the relative path to sibling repos. The correct path is `../unshit-rust-framework/crates/unshit` from the repo root.

2. **Run `cargo run` after merging agent work.** Tests passing does NOT mean the app works correctly. Visual regressions (layout, CSS) are not caught by unit tests. Always launch the app and verify visually.

3. **Merge agents one at a time, not all at once.** Parallel agent branches that touch overlapping files create cascading merge conflicts. Merge and verify each one sequentially.

4. **Check for missing callbacks after conflict resolution.** `AppConfig` fields like `on_close`, `on_scale_factor`, `on_cell_metrics` can be silently dropped during conflict resolution. Verify all callbacks are present after merging.

5. **Do not remove eager PTY spawn.** The initial PTY spawn in `main.rs` is load-bearing. Without it, the terminal never renders.

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
