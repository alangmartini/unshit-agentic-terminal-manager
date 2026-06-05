# CSS engine: text-overflow: ellipsis

## Added

- `text-overflow: ellipsis` is now honored. Single-line labels that overflow
  their box (the `overflow:hidden; white-space:nowrap; text-overflow:ellipsis`
  triad used on ~15 stylesheet sites — command-palette result labels/sublabels,
  sidebar workspace/session names, tab labels, theme-chip names) now truncate
  with a trailing `…` instead of being hard-clipped mid-glyph by the clip rect.
  Adds a non-inheriting `TextOverflow {Clip, Ellipsis}` computed-style field + a
  `text-overflow` parse arm; the renderer truncates before glyph emission when
  `Ellipsis && white-space:nowrap` and the run overflows. No GPU/shader change.

## Notes

- The truncation fit is decided authoritatively against what is actually
  **painted**: `truncate_text_with_ellipsis` iterates **logical** grapheme-cluster
  boundaries and keeps the largest `prefix + …` whose **painted** width
  (`painted_run_width`, the renderer's exact
  `glyph.x + glyph_index*letter_spacing + glyph.w`) fits `content_w`. This is
  correct for LTR, RTL, bidi, combining-mark and ZWJ-emoji text at any
  letter-spacing — an earlier visual-order walk overflowed bidi labels by up to
  ~5× and clipped the ellipsis on letter-spaced combining-mark text; both are
  covered by a fit-invariant test matrix (script-mix × letter-spacing × width).
- Accepted limitation: for RTL runs this truncates the **logical tail** rather
  than the CSS-perfect visual-left end. Fit is always guaranteed and the result
  is always a valid logical prefix + ellipsis. Tracked in
  `specs/css-engine-stylesheet-gaps.md`.
