# Repository Instructions

## Project Shape

This is a Rust terminal manager with a PTY daemon backend and web frontend, built on the local `unshit` framework subtree.

- App code lives in `src/`.
- UI assets live in `assets/`.
- The framework lives in `crates/unshit-framework/`.
- The PTY session daemon lives in `crates/unshit-ptyd/`.

## Architecture Rules

- Prefer framework-level fixes for framework-level problems. If behavior belongs in a general UI toolkit, fix it in `crates/unshit-framework/` instead of patching around it in app code.
- Keep app-level fixes for app-specific behavior: PTY lifecycle, terminal emulation wiring, state management, bridge code, and product-specific layout.
- Do not remove eager PTY spawning in `main.rs`. The terminal must spawn at a default size first, then correct dimensions once cell metrics are available.
- The UI reattaches to daemon-owned sessions. Sessions live in `unshit-ptyd`, not in the UI process.

## Performance Invariants

- Do not add synchronous IPC to the render path. `DaemonPty::write()` must remain fire-and-forget; blocking APIs are only for tests or setup.
- Cursor blink should be renderer-side and use redraws, not full tree rebuilds, for quiet blink ticks.
- Resize and cell dimension synchronization should prefer redraws over rebuilds unless tree state actually changed.
- Preserve rebuild coalescing behavior in the framework.

## Framework Subtree

The framework was imported as a git subtree from `unshit-rust-framework`.

```bash
git subtree pull --prefix=crates/unshit-framework unshit-upstream master --squash
git subtree push --prefix=crates/unshit-framework unshit-upstream <branch-name>
```

## Quality Gates

For code changes, run the smallest useful verification locally, then broaden based on risk.

- `cargo fmt --check`
- `cargo clippy -- -D warnings`
- `cargo test`
- Relevant benches when touching hot paths, especially `pty_write` and `quick_prompt_filter`.
- Launch the app with `cargo run` for UI/layout-sensitive changes when practical.

Bug fixes should include a regression test when the behavior can be tested without excessive scaffolding.

## Git And Review Rules

- Keep commits atomic and use conventional prefixes such as `feat:`, `fix:`, `test:`, `refactor:`, `docs:`, `style:`, or `chore:`.
- Do not add AI attribution, assistant signatures, or yourself as a committer/co-author.
- Do not rewrite, reset, or revert user changes unless explicitly asked.
- When resolving merges around app configuration, verify callbacks such as `on_close`, `on_scale_factor`, and `on_cell_metrics` are still wired.
- Merge parallel or agent-produced work one branch at a time and verify after each merge.

## Task Response Guidance

- After finishing each task, include a short recommendation section for the next likely improvement, feature, or fix whenever one is clearly useful.
