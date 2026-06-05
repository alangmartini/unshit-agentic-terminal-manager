# CSS engine: full `transform` (scale / rotate / translate, animated)

## Added

- CSS `transform` now honors `scale` / `scaleX` / `scaleY`, `rotate`,
  `translate` / `translateX` / `translateY`, and the combined
  `translateY(..) scale(..)` form (previously only `translateX` applied; every
  other function silently dropped). `transform: none` is an explicit identity.
  The functions compose into a per-element 2x3 affine about the box center
  (transform-origin defaults to `50% 50%`), which is propagated to the whole
  subtree — so e.g. `.icon-btn:active { transform: scale(0.94) }` scales its
  icon, the chevron's `rotate(-90deg)` turns, and the modal-in keyframe's
  `translateY(-12px) scale(0.98)` and the card-lift `translateY` now actually
  move. Both transitions and `@keyframes` interpolate the transform
  component-wise.

## Changed

- The renderer carries the transform as a delta-from-identity affine on each
  `QuadInstance` / `GlyphInstance` (`xform` 2x2 + `xform_translate`), applied to
  the screen-space position in the quad and the three text vertex shaders before
  NDC. The fragment shaders are unchanged: in-quad coordinates (`local_pos`,
  used for the border-radius SDF and gradient projection) stay untransformed and
  rotate / scale *with* the quad, while only the clip-test position (`pixel_pos`)
  is transformed against the axis-aligned ancestor clip. Untransformed elements
  encode all-zero and stay on the matrix-free fast path. The batch cache
  signature includes the composed affine, so a node re-emits when its own or any
  ancestor's transform changes. The previous scalar `translateX`-only render
  offset (`transform_dx`) is retired.

## Notes

- Accepted limitations (both pre-existing-grade, none a regression from the old
  `translateX`-only path): a descendant's `overflow` clip is computed in the
  element's untransformed space, so a *rotated* clipping ancestor can mis-clip
  mid-animation (exact at rest, since animated transforms rest at identity); and
  the `<select>` dropdown overlay (a separate top-level paint pass) is not
  transformed by an ancestor's affine — the old mechanism didn't transform it
  either, and no open dropdown sits under a transform at rest. The app authors no
  `transform-origin`, `matrix()`, `skew()`, or 3D functions;
  those remain unsupported (a list containing one drops the whole declaration so
  the `stylesheet_coverage` guardrail still flags it). Covered by affine-math
  unit tests, parse tests for every authored form, and GPU render tests for
  scale / translateY / subtree propagation; `transform` is removed from
  `KNOWN_UNSUPPORTED`.
