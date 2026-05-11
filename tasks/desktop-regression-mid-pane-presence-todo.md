# Todo: Desktop Regression Mid-Pane Presence

- [x] Add failing classification and assertion tests.
- [x] Implement `snap-mid-pane-blank` classification and lower-bound assertion.
- [x] Add threshold calibration note near the threshold.
- [ ] Run focused tests, build, and format check.
- [x] Review and simplify the changed code.
- [x] Record ship decision and rollback plan.
- [x] Create changelog fragment if GO.

Validation note: `cargo test -p xtask desktop_regression` and
`cargo build -p xtask` pass. The final `cargo fmt --check` is blocked by
unrelated concurrent formatting diffs outside this closure and by
snap-composition hunks in the same Rust file.

Ship note: GO for this closure with the formatter caveat above.
