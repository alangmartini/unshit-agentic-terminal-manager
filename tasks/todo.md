# TODO: Desktop Regression Observability Harness

Implementation checklist derived from `tasks/plan.md` and `specs/desktop-regression-observability-harness.md`. Keep each task small, test-first, and independently reviewable.

## Phase 1: Protocol And Safety Foundation

- [x] **Task 1: Create shared diagnostic schema crate**
  - [x] Add `crates/terminal-manager-diagnostics` as a workspace member.
  - [x] Define version constants and serde types for commands, responses, envelopes, capabilities, events, snapshots, invariants, actions, results, and failure manifests.
  - [x] Add compatibility helpers for supported protocol/schema versions.
  - [x] Add JSON round-trip tests for commands, events, snapshots, results, and failure manifests.
  - [x] Verify: `cargo test -p terminal-manager-diagnostics`.
  - [x] Verify: `cargo fmt --check`.

- [x] **Task 2: Add gated app diagnostic handshake**
  - [x] Add `src/diagnostics/` config and server modules.
  - [x] Require explicit enablement, pipe name, and token before listening.
  - [x] Implement `hello` handshake with protocol version, pid, build identity, features, and capabilities.
  - [x] Reject missing or invalid tokens.
  - [x] Prove default app launch exposes no diagnostic endpoint.
  - [x] Verify: app diagnostic handshake tests.
  - [ ] Verify: manual launch with and without diagnostics env vars.

- [x] **Task 3: Expose app snapshots and invariants**
  - [x] Add snapshot collectors for initial window/layout/terminal/renderer/PTY/input/config/log metadata.
  - [x] Exclude terminal buffer contents by default.
  - [x] Add invariant ids and pass/fail/skipped results.
  - [x] Add `snapshot`, `evaluate_invariants`, and `prepare_deterministic_mode` command handling.
  - [x] Return structured protocol errors for collector failures.
  - [x] Verify: snapshot/invariant serde and collector tests.
  - [x] Verify: handshake + snapshot + invariant protocol test.

### Checkpoint: Protocol Foundation

- [ ] Human review diagnostic enablement gates and token/pipe contract.
- [ ] `cargo test -p terminal-manager-diagnostics` passes.
- [ ] App diagnostic tests pass.
- [ ] Manual default launch has no diagnostic endpoint.

## Phase 2: Events, Logs, And Runner Skeleton

- [x] **Task 4: Add structured app events, step markers, and flush**
  - [x] Add ordered diagnostic event queue with sequence numbers and timestamps.
  - [x] Emit initial event families: `test.step`, `window`, `layout`, `render`, `terminal`, `pty`, `input`, `invariant`, `log`.
  - [x] Implement `mark_step` correlation.
  - [x] Implement `flush`.
  - [x] Add cap or dropped-event counter.
  - [x] Verify: event sequencing and JSONL tests.
  - [x] Verify: protocol test for mark step, event emission, and flush.

- [x] **Task 5: Add `xtask desktop-regression` list and result skeleton**
  - [x] Add CLI parsing for `desktop-regression`.
  - [x] Support `--list`, `--suite`, `--skip-build`, `--exe-path`, `--observe`, `--interactive`, `--keep-open-on-failure`, `--record`, and artifact root.
  - [x] Add Rust suite registry with metadata.
  - [x] Add run id and artifact directory creation only for real runs.
  - [x] Write v2 `results.json` with shared schema.
  - [x] Verify: `cargo test -p xtask desktop_regression`.
  - [x] Verify: `cargo run -p xtask -- desktop-regression --list`.
  - [x] Verify: missing suite fails before artifacts/app launch.

- [x] **Task 6: Run first Rust black-box desktop suite**
  - [x] Add Rust app build/launch path with `--skip-build` and `--exe-path`.
  - [x] Add Win32 window lookup by process id and title/class fallback.
  - [x] Add window move/resize/focus/global input helpers.
  - [x] Add screenshot capture and rectangle assertion helpers.
  - [x] Port `edge-resize-stability` for `--observe off`.
  - [x] Write pass/fail results and screenshot artifacts.
  - [x] Verify: `cargo test -p xtask desktop_regression`.
  - [x] Verify manually on Windows: `cargo xtask desktop-regression --suite edge-resize-stability --skip-build --exe-path target\\debug\\terminal-manager.exe --observe off`.

### Checkpoint: Runner Black-Box Path

- [ ] Human review Rust CLI and result schema.
- [ ] `--list` works without app launch.
- [ ] `edge-resize-stability` runs with `--observe off`.
- [ ] Existing PowerShell runner still works.

## Phase 3: Observed Suite And Failure Evidence

- [ ] **Task 7: Add app launcher logs and basic failure bundles**
  - [ ] Capture app stdout/stderr to artifact files.
  - [ ] Write runner JSONL logs.
  - [ ] Capture environment metadata, binary path/hash, source commit, and dirty-worktree metadata.
  - [ ] Write final screenshot and failure manifest on suite failure.
  - [ ] Preserve original error and record diagnostic capture errors separately.
  - [ ] Link bundle artifacts from `results.json`.
  - [ ] Verify: failure classification and manifest unit tests.
  - [ ] Verify: intentional failure writes expected bundle.

- [ ] **Task 8: Connect runner diagnostics for one observed suite**
  - [ ] Add Rust diagnostic client.
  - [ ] Wire observe modes into app launch environment.
  - [ ] Keep `--observe off` diagnostic-free.
  - [ ] Capture events/logs/failure snapshots in `--observe basic`.
  - [ ] Capture step snapshots, invariants, and cross-layer assertions in `--observe full`.
  - [ ] Add required/optional diagnostic capability checks.
  - [ ] Verify: diagnostic client and observe-mode unit tests.
  - [ ] Verify manually: `cargo xtask desktop-regression --suite edge-resize-stability --observe full`.

- [ ] **Task 9: Port `post-resize-glitches` with cross-layer evidence**
  - [ ] Port Win+Left snap and pixel-ratio assertions to Rust.
  - [ ] Preserve black-box `--observe off` behavior.
  - [ ] Add full-observe before/after snapshots and event correlation.
  - [ ] Add cross-layer checks for window bounds, app surface/layout, and terminal dimensions.
  - [ ] Add first-bad classification for resize/render/pixel failures.
  - [ ] Verify manually: `cargo xtask desktop-regression --suite post-resize-glitches --observe off`.
  - [ ] Verify manually: `cargo xtask desktop-regression --suite post-resize-glitches --observe full`.

### Checkpoint: Observability Harness

- [ ] Human review artifact shape and failure bundle contents.
- [ ] Both migrated suites run in `--observe off`.
- [ ] At least one suite runs in `--observe full` with snapshots and cross-layer assertions.
- [ ] Forced failure bundle contains screenshots, stdout/stderr, app events, runner events, snapshot, environment metadata, binary hash, and failure manifest.

## Phase 4: Migration Compatibility And Advanced Debugging

- [ ] **Task 10: Preserve PowerShell compatibility wrappers and docs**
  - [ ] Keep `tests/windows/desktop-regression/run.ps1 -List` non-invasive.
  - [ ] Map PowerShell `-Suite`, `-SkipBuild`, and `-ExePath` to Rust where coverage exists.
  - [ ] Clearly report legacy-only suites if any remain.
  - [ ] Update README and framework spec with Rust commands, observe modes, artifacts, and compatibility behavior.
  - [ ] Validate historical wrapper paths if present.
  - [ ] Verify: PowerShell `-List`.
  - [ ] Verify: PowerShell one-suite skip-build path.
  - [ ] Verify: `cargo xtask desktop-regression --list`.

- [ ] **Task 11: Add record and replay for runner actions**
  - [ ] Write versioned action trace JSONL for `--record`.
  - [ ] Include action kind, timestamps, target metadata, coordinates, keys, step id, and wait mode.
  - [ ] Add replay command/path that validates trace schema before app launch.
  - [ ] Distinguish exact timed replay from logical replay in results.
  - [ ] Reject unsafe or unknown trace actions.
  - [ ] Verify: trace serialization/validation tests.
  - [ ] Verify manually: record and replay `edge-resize-stability`.

- [ ] **Task 12: Add interactive keep-open-on-failure workflow**
  - [ ] Implement `--interactive --keep-open-on-failure` pause after failure.
  - [ ] Add prompt commands for snapshot, event tail, screenshot, rerun last assertion, note, continue, abort, and close.
  - [ ] Store notes in the failure bundle.
  - [ ] Ensure non-interactive failures never wait for input.
  - [ ] Make cleanup behavior explicit for continue/abort/close.
  - [ ] Verify: interactive command parser and note artifact tests.
  - [ ] Verify manually: forced failure interactive workflow.

### Checkpoint: Migration Complete Enough For Review

- [ ] Human review replay and interactive command surface.
- [ ] Rust commands cover list, both migrated suites, observe off/basic/full, skip-build, explicit exe path, record, and interactive failure pause.
- [ ] PowerShell compatibility paths forward or document legacy behavior.
- [ ] Failure bundles are inspectable locally without remote services.
- [ ] Production/default launches do not expose diagnostics.

## Final Validation

- [ ] `cargo xtask desktop-regression --list` lists migrated suites without launching the app.
- [ ] `cargo xtask desktop-regression --suite edge-resize-stability --observe off` drives the desktop and writes results.
- [ ] `cargo xtask desktop-regression --suite post-resize-glitches --observe full` captures black-box and diagnostic evidence.
- [ ] `cargo test -p terminal-manager-diagnostics` passes.
- [ ] `cargo test -p xtask desktop_regression` passes.
- [ ] Existing PowerShell command paths still work as wrappers or clearly direct users to Rust.
