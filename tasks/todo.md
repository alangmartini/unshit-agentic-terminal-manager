# File Editor MVP Todo

- [x] S1: Read-only file viewer pane (structural seam)
  - Acceptance: `editor.open:<path>` creates tab+pane+editors entry; grid renders
    viewport-only with gutter and stable line_ids; oversized/invalid-UTF-8 refused
    with notification; terminal panes unaffected; open telemetry events emitted.
  - Verify: `cargo test -p terminal-manager editor`; full tests; fmt; clippy.
- [ ] CHECKPOINT A: manual launch (isolation profile), open src/state.rs, scroll;
      screenshot; terminals still work.
- [x] S2: Core editing (buffer ops, keyboard capture, cursor/selection render, dirty)
  - Acceptance: buffer unit suite (edits/movement/selection/UTF-8/sticky column);
    typing "abc" via capture handler lands in grid cells.
  - Verify: `cargo test -p terminal-manager editor_buffer` + editor UI tests.
- [ ] S3: Clipboard + undo/redo
  - Acceptance: grouping, cursor restore, redo-clear, multi-line clipboard tests.
  - Verify: focused buffer tests.
- [ ] S4: Save + open UX (atomic save, CRLF preserve, rfd dialog, keybinds, palette)
  - Acceptance: save/failure/CRLF dispatch tests; keybind parity green; palette rows.
  - Verify: `cargo test -p terminal-manager keybind` + editor tests.
- [ ] CHECKPOINT B: full tests + fmt + clippy; manual edit+save on real file; split
      editor beside terminal.
- [ ] S5: Mouse (click cursor, drag select, double-click word, wheel, gutter)
  - Acceptance: click→cursor and drag-selection UI tests.
- [ ] S6: Lifecycle polish + full guard audit + telemetry completeness
  - Acceptance: close-dirty dialog tests; terminal-only dispatch no-op tests;
    privacy test on telemetry sink; diagnostics events recorded.
- [ ] CHECKPOINT C: full gates, manual smoke + screenshot, telemetry inspected,
      changelog fragment, commit(s).
