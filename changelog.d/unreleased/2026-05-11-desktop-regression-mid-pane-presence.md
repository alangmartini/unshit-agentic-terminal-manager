# Desktop Regression Mid-Pane Presence

## Fixed

- Made the `post-resize-glitches` desktop regression suite fail with
  `snap-mid-pane-blank` when the post-snap terminal pane is visually blank,
  while preserving the existing bottom-stripe and stale-row failure signals.

## Notes

- The new mid-pane presence threshold is conservative and should be
  re-baselined when terminal theme, foreground palette, antialiasing, sample
  geometry, or lit-pixel thresholds change.
