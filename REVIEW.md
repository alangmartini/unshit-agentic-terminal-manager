# Code Review Rules

## Always Flag

- Missing tests for new features (unit, integration, or e2e)
- Bug fixes without regression tests
- `unwrap()` or `expect()` in non-test code without justification
- Unsafe blocks without a safety comment
- Code that could be simplified (unnecessary abstractions, duplicated logic)
- Decreasing test coverage

## Never Flag

- Style preferences already handled by `cargo fmt`
- Import ordering (handled by tooling)
- Comment style (unless comments are misleading)
