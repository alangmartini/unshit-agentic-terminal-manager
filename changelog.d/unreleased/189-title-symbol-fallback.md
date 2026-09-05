### Fixed

- **Status symbols in sidebar rows and tab titles.** Labels rendered a solid
  box where a guest window title carried `✳` (Claude Code's idle title
  prefix) or another text-presentation symbol the UI font lacks. The
  platform font fallback resolved such characters to the color-emoji face,
  whose glyphs the renderer can only flatten into their silhouette; they are
  now re-shaped onto the monochrome symbol face, so `✳ Workspace` reads as
  intended while explicit emoji sequences (`✳️`) and full emoji are left
  unchanged. Text measurement and the terminal grid use the same shaping,
  and the fallback rate is recorded as `renderer.symbol_fallback` (counts
  only) in the profile's `renderer-events.jsonl`.
