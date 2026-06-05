# Command Palette Redesign

## Added

- Added a VS Code-style command palette on `Ctrl+Shift+P`, with grouped
  results, keyboard/mouse selection, preview details, footer hints, and modes
  for commands (`>`), agents (`@`), navigation (`:`), and scrollback (`/`).
- Added safe palette actions for renaming the current terminal, splitting panes
  right or down, opening a new terminal, closing the current pane, toggling the
  sidebar, and opening settings.
- Added real-data-only palette rows from the current UI snapshot, including
  honest empty states for agent and scrollback modes when no source data is
  available.

## Changed

- Made `Ctrl+Shift+P` the editable default command-palette keybind while
  retaining `Ctrl+K` as an alias, so settings can override the effective
  shortcut.
- Updated palette keyboard handling so the palette owns input while open,
  supports multi-term searches, and prevents terminal capture or global
  shortcuts from running behind the modal.
- Updated palette result rendering to sanitize long or control-heavy labels,
  show disabled metadata rows as non-executable, and present explicit action
  effects in the preview.

## Fixed

- Fixed the palette not matching its design. Three causes: (1) the `--cp-*`
  accent/density tokens were declared on `.cp-scrim`, but the stylesheet engine
  only collects custom properties from `:root`/`*`, so every `var(--cp-*)`
  consumer (active-row amber fill, row padding, prompt color) was silently
  dropped — the tokens now live in `:root`; (2) the active-row left rail used
  `border-left-color`, and (3) the card's `12vh` top offset used the scrim's
  `padding-top: 12vh` — both unsupported until the engine upgrades below, after
  which the palette uses the design's CSS verbatim (no positioning hacks). The
  horizontal centering (scrim `justify-content: center`) always worked; an early
  misdiagnosis came from a DPI-unaware screenshot, now corrected.

## Notes

- The local design handoff for the palette was saved under
  `docs/design/command-palette-handoff/`.
- Added `scripts/palette-shot.ps1` to launch the app, open the palette, and
  capture a DPI-correct screenshot for visual regression checks.
- The engine capabilities this relies on are tracked in
  `2026-06-04-css-engine-viewport-units-and-border-side-colors.md`.
