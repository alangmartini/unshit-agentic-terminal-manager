# CSS engine: viewport units in padding + per-side border colors

## Added

- `border-top-color` / `border-right-color` / `border-bottom-color` /
  `border-left-color` longhands are now parsed. They write the single stored
  `border_color`, which is visually exact whenever one side has width (the case
  for every authored consumer: theme-chip dividers, pane-header underline,
  setting-row hover, the command-palette active rail, etc.). These declarations
  were previously unrecognized and silently dropped.
- `padding` (shorthand and longhands) now accepts viewport and percent units
  (`vh` / `vw` / `%`). Viewport units resolve against the viewport in
  `to_taffy_style`; percent stays taffy-native. Pure-`px` padding keeps its
  existing resolved-`f32` fast path, so paint, hit-testing, and transitions are
  unchanged. This makes the design's top-anchored overlays (`.cp-scrim` /
  `.quick-prompt-overlay` `padding-top: 12vh`/`14vh`) lay out as authored
  instead of dropping the declaration and pinning to the top.
- Coverage diagnostic: `CompiledStylesheet` now records every declaration the
  parser could not type (`dropped: Vec<DroppedDeclaration>`) instead of silently
  discarding it. A dev-build log summarizes them on startup, and the new
  `stylesheet_coverage` guardrail test fails the build if the app stylesheet
  grows a gap outside the documented inventory — so the next unsupported
  property surfaces immediately, not when a screenshot looks wrong.

## Fixed

- A comment between `:root` declarations (especially one containing `:`) broke
  collection of the custom property immediately after it, because the
  custom-property pre-scan split the block on `;`/`:` without stripping
  comments. This silently dropped `--cp-accent`, so the command-palette prompt
  glyph and active-row rail fell back instead of rendering amber. The pre-scan
  now strips comments first. (Caught by the new coverage guardrail.)

## Notes

- Internals: `ComputedStyle` gains a unit-preserving `padding_src: EdgesDim`
  kept in sync with the resolved `padding`; `to_taffy_style` resolves it against
  the viewport (re-resolved on every layout, including resize). For non-`px`
  units the `f32` `padding` mirror stays `0` — harmless because the only
  consumers are content-less full-viewport scrims, where padding only affects
  the flex layout taffy already computes from the source.
- Known remaining stylesheet-engine gaps, deliberately deferred (designs drafted
  but judged too risky / low-value for this change): cascade-aware custom
  properties resolved per element (today `var()` is a global parse-time text
  substitution seeded only from `:root`, so `.app.theme-*` token overrides are
  dropped and theming uses concrete declarations); `calc()`; and the non-backdrop
  `filter: drop-shadow()` (the active-row icon glow).
