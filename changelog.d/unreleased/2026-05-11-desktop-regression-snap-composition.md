# Desktop Regression Snap Composition Guard

## Fixed

- Updated the `post-resize-glitches` desktop regression suite to fail before
  post-snap screenshot capture when the terminal-manager window loses
  foreground, a modifier remains stuck after `Win+Left`, or another visible
  non-owned top-level window overlaps the snapped window.

## Notes

- Full headed verification remains manual because the suite controls the real
  Windows desktop and captures live screenshots.
