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
- `src/bridge.rs` - Frontend-backend bridge
- `src/state.rs` - Application state
- `src/ui/` - UI components (split panes, settings, keyboard nav)
- Root (`index.html`, `app.js`, `styles.css`) - Web frontend
- Framework dependency: `../unshit-rust-framework/`

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
