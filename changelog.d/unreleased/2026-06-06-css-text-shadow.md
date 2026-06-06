# CSS engine: `text-shadow` (colored glow)

## Added

- CSS `text-shadow` now paints. The full syntax parses (`<color>? <offset-x>
  <offset-y> <blur>?`, comma-separated layers, flexible color position; `none`
  clears), mirroring `box-shadow`; an omitted color defaults to `currentColor`,
  and `var()` colors resolve through the cascade (so the prompt's
  `0 0 6px var(--accent-a35)` picks up the active theme). The app's three
  authored glows now render — the active workspace name, the shell prompt, and
  the search highlight — where they previously dropped.

## Notes

- The glow is rendered **without render targets**, reusing the existing text
  pipeline and shaders unchanged: the glyph run is re-drawn a handful of times
  behind the text at small offsets sampled on a Gaussian-weighted disc (a Vogel
  spiral out to the blur radius), in the shadow color at a weighted fraction of
  its alpha, so the overlapping copies sum into a soft halo. This sidesteps both
  the heavy offscreen-blur path and the glyph-atlas padding limit (each copy
  samples its own glyph entry; only the quad position is offset). The result is
  a faithful soft glow for the subtle (alpha 0.2–0.32, 6–8px) shadows the app
  uses; it is a close approximation rather than a pixel-exact Gaussian, which is
  imperceptible at these alphas. Tap count scales with the blur radius and is
  bounded, and blur is clamped to `[0, 64]`, so the extra glyph instances stay
  small (the shadowed elements are short labels). Shadow instances live in the
  node's glyph range, so they cache, replay, and inherit the node's CSS
  `transform` like the main text.
- `text-shadow` is removed from `KNOWN_UNSUPPORTED`. Covered by parser unit
  tests (every authored form + `none` + currentColor default) and a GPU render
  test (a colored glow paints a broad halo of shadow-colored pixels around the
  text that bare text does not, and `none` adds none). Offsets and multiple
  layers are supported though the app authors only zero-offset single glows;
  blurred-glow quality, not sharp drop-shadows, was the target.
