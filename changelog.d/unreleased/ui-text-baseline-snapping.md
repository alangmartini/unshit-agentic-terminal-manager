### Fixed

- UI/chrome text (sidebar, tabs, breadcrumbs, status bar, buttons) now snaps its glyph baseline to a whole device-pixel row, so horizontal stems land on one crisp row instead of smearing across two at partial coverage on non-integer display scales (e.g. 1.5x). Positions are already in device pixels (font sizes are pre-scaled by the DPR); only Y is rounded (X is left untouched to preserve shaping/kerning), mirroring the trick the terminal grid path already uses (`gy.round()`). This path is UI-only — terminal cells render through their own emit path and are unaffected.
