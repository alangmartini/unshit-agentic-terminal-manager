# CSS engine: close the tier-1 stylesheet gaps

These properties were authored in `assets/styles.css` but silently dropped by
the engine (surfaced by the `stylesheet_coverage` guardrail). Each is now parsed
and resolved using the renderer's existing paint paths — no new GPU/shader work.
The known-gap inventory shrinks from ~28 properties to 11 (the genuinely
render/text-layout-bound ones remain documented and deferred).

## Added

- `border-radius` now accepts percentages (`50%`). Corners are kept
  unit-preserving (`border_radius_src: CornersDim`) and resolved against the box
  at paint time (`min(width, height)`, so `50%` stays circular on non-square
  boxes), exactly mirroring the `padding` / `padding_src` dual-write and
  `RadialGradient::resolve` patterns. Pure-`px` corners keep their `f32` fast
  path, so transitions and scaling of px radii are unchanged. The rounded-rect
  SDF shader already drew per-corner radii, so this is purely parse + resolve.
- Per-axis `overflow-x` / `overflow-y`. The single `overflow` field is split into
  `overflow_x` / `overflow_y`; the `overflow` shorthand now sets both. The clip
  rect tightens each axis independently and the scrollbar / resize-grip gates are
  per-axis, so `overflow-x: hidden; overflow-y: auto` (and the reverse) lay out
  and clip as authored instead of dropping.
- `outline` shorthand (`outline: 1px solid <color>`, `outline: none`). Parsed
  order-independently like the `border` shorthand (width / color / style keyword
  in any order; `none`/`hidden` → width 0), reusing the existing
  `outline-color` / `-width` / `-offset` fields and paint path.
- `font-style: italic` (and `oblique`). A new inherited `font_style` field is
  threaded through the text measure/shape cache keys and sets cosmic-text
  `Style::Italic` plus the `FAKE_ITALIC` cache-key flag, so the render-time skew
  that already shipped for terminal cells now applies to DOM text.
- `justify-content: stretch` (plus `left` / `right` aliased to `start` / `end`).
- `background: none` (resolves to transparent); a comma-separated multi-layer
  background keeps its first paintable layer instead of dropping.
- `transition` no longer drops the **entire** declaration when the list names
  `transform` (or any not-yet-animatable property). `transform` is now an
  animatable `TransitionProperty` whose render-applied `translateX` component
  animates (and animates from `@keyframes` too); unrecognized transition
  properties are skipped individually rather than failing the whole list.

## Changed

- Recognized-but-inert: `appearance` / `-webkit-appearance`,
  `-webkit-font-smoothing`, `border-collapse`, `border-style` (non-`none`
  styles; `none`/`hidden` collapse the border to zero width),
  `background-repeat`, `font-feature-settings`, `font-variant-numeric`,
  `scrollbar-width`, and `text-shadow: none` are now accepted and intentionally
  ignored (they have no render target in this engine), matching CSS
  forward-compat semantics instead of dropping. Real (non-`none`) `text-shadow`
  values still drop, so the guardrail keeps surfacing them.
- `caret-color` was removed from the known-gap inventory: its stylesheet values
  are `var()` references that resolve to hex and already parsed — the entry was
  stale (no parser change needed).

## Notes

- Still deferred (genuinely renderer/text-layout-bound, kept in
  `KNOWN_UNSUPPORTED` with reasons): `transform: scale/rotate/translateY` (needs
  an affine matrix threaded through the quad + text instances and shaders),
  `mix-blend-mode` (per-pipeline blend), `filter: drop-shadow` (offscreen alpha
  pass), `text-overflow` / `word-break` / `vertical-align` (text-layout
  machinery), multi-layer `background-position` / `-size` and the
  `ellipse … at …` radial-gradient form (per-layer paint loop / gradient-parser
  gap), `calc()` (evaluator), and `inherit` (per-property cascade lookup).
- Cleanup: moved `scroll.rs`'s `visual_state_tests` module below the functions it
  precedes so `cargo clippy --all-targets -- -D warnings` (which compiles the
  test target) is clean.
