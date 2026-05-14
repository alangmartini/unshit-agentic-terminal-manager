# Desktop Regression Diagnostic Producer Fields

## Added

- Added producer-side diagnostic snapshot data for live terminal cursor,
  scrollback length, active session id, PTY session mappings, content-free PTY
  liveness events, renderer frame counter, renderer last-present time, and
  renderer-facing dirty cell regions.
- Added an authenticated opt-in terminal `buffer_window` snapshot field gated by
  `include_terminal_buffer`; default snapshots continue to exclude terminal
  contents.

## Changed

- Updated desktop-regression debugging docs to describe the wired snapshot
  fields and to keep event-family caveats limited to event streams that are
  still not emitted.

## Notes

- `renderer.glyph_atlas` remains absent until the app exposes truthful atlas
  page/glyph state. Headed desktop suite execution remains manual.
