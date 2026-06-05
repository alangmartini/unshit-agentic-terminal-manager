# Spec: CSS engine — remaining stylesheet gaps

## Objective

Track the CSS declarations that `assets/styles.css` authors but the `unshit`
engine does not yet fully honor, so the work to close them is discoverable and
prioritized instead of living only in ephemeral changelog notes.

The **live, build-enforced inventory** is `KNOWN_UNSUPPORTED` in
`tests/stylesheet_coverage.rs`. The `stylesheet_has_no_unknown_engine_gaps`
guardrail fails the build if a *new* dropped declaration appears outside that
list (plus the `calc()` / `inherit` value-form allowances). This spec is the
narrative companion: the **why**, **effort**, and **plan** behind each entry.
When an item lands, remove it from `KNOWN_UNSUPPORTED` and from the relevant
table row here.

## Status (2026-06-05)

- **Tier 1 — landed** (see `changelog.d/unreleased/2026-06-05-css-engine-tier1-stylesheet-gaps.md`):
  17 properties cleared via existing renderer paint paths — `border-radius` %,
  `overflow-x/-y`, `outline` shorthand, `font-style`, `justify-content: stretch`,
  `background: none`, `transition` transform coverage, and 9 recognized-but-inert
  accepts. Inventory shrank from ~28 to the entries below.
- **Tier 2 — deferred:** the table below. Each is genuinely renderer-,
  text-layout-, value-evaluator-, or cascade-bound — not a one-line parse arm.

## Enforcement coverage (what the guardrail does / does not catch)

- **Caught (build fails on a new gap):** every per-property entry in
  `KNOWN_UNSUPPORTED`, plus any new `calc()` value (`is_known_gap` matches
  `value.contains("calc(")`) and `inherit` (`value == "inherit"`).
- **NOT caught (by design):** custom-property definitions on non-`:root`
  selectors (every `.app.theme-*` token override) are *counted but not failed*
  (`custom_count`). A new theme custom-property override will not fail the build.
  This is the cascade-aware-custom-properties gap (last row).

## Deferred inventory

| Property / form | Drops today | Class | Effort | Value |
|---|---|---|---|---|
| `transform: scale/rotate/translateY` | `scale/rotate/translateY` drop; only `translateX` is applied | renderer (affine) | L | H |
| `text-overflow: ellipsis` | no truncation hook; clip rect cuts glyphs mid-pixel | text-layout | L | H |
| `text-shadow` (non-`none`) | `none` accepted; real shadow lists drop | small-render | M | M |
| `filter: drop-shadow(...)` | no element `filter` field; only `backdrop-filter` blur exists | renderer (offscreen) | L | M |
| `word-break: break-word` | no `set_wrap` control in the shaper | text-layout | M | M |
| `mix-blend-mode: multiply` | blend is baked per-pipeline, not per-instance | renderer (blend) | L | L |
| `vertical-align: text-bottom` | no inline/baseline layout model | text-layout | L | L |
| `background-position` / `background-size` | single `Background` field; gradients fill the box | small-render | M | L |
| `background` (`ellipse <size> at <pos>`) | radial-gradient parser rejects this form | parse (gradient) | M | L |
| `scroll-margin` | no scroll-snap / anchor consumer | parse (inert) | S | L |
| `calc()` (value form) | no evaluator in leaf parsers | value-form | L | M |
| `inherit` (value form) | no per-property cascade keyword | value-form (cascade) | M | M |
| `.app.theme-*` `--token` overrides | `var()` is a global parse-time substitution seeded only from `:root` | cascade | L | M |

## Plans (highest value first)

### `transform: scale` / `rotate` / `translateY`  — renderer (affine)
The paint path applies only a scalar x-offset (`transform_dx`, `batch.rs:~1411`);
`QuadInstance` / `GlyphInstance` carry no transform matrix. Needs a `mat2x2` /
`[f32;6]` instance field threaded through `QuadInstance` + the vertex attr array
(`pipeline/quad.rs`) + the WGSL `QuadInstance`/`VertexOutput`/`vs_main` (transform
the corner before NDC), a parallel `GlyphInstance`/text-shader change, a
`transform-origin` field, and rework of the per-fragment `clip_rect` test +
gradient projection (`quad.wgsl:~142`), which assume an axis-aligned quad
(rotation breaks both). **Quick win inside this cluster:** `translateY` is a
scalar `dy` symmetric to the existing `dx` (add `transform_translate_y`, a
`transform_dy` at `batch.rs:~1411` applied to `render_y`, child-scroll-y
propagation) with no shader change — but `transform` only leaves
`KNOWN_UNSUPPORTED` once `scale` + `rotate` also land. Couples with the
`transition` transform animation already wired in tier 1.

### `text-overflow: ellipsis`  — text-layout (top tier-2 candidate)
Most common dropped property in the stylesheet (8+ sites; it is the canonical
`DroppedDeclaration` test). Add a `text_overflow: TextOverflow` (Clip/Ellipsis)
field + a `"text-overflow"` arm, then the real work: measure the shaped run
against `content_w`, find the break cluster, truncate, and append `…` before
glyph emission — `layout.rs` measure + the emit site `batch.rs:~2061-2174`. No
GPU change. Do **not** add a parse-noop: that would mask the missing ellipsis on
the many list rows that use it.

### `text-shadow` (real, non-`none`)  — small-render
`none` is already accepted (tier 1). Sharp/offset shadows: re-emit the glyph run
at an offset with the shadow color before the main run (second
`emit_text_glyphs_cached` at `batch.rs:~2156`), reusing atlas coverage — no
shader change. Needs a `text_shadow` field + `parse_text_shadow_list` modeled on
`parse_box_shadow_list` (`parse.rs:~3229`). Blurred shadows are large-render
(offscreen + blur).

### `filter: drop-shadow(...)`  — renderer (offscreen)
Drop-shadow over arbitrary content needs an offscreen-of-own-content alpha pass +
blur + composite (different boundary than `backdrop-filter`, which blurs what is
*behind*). `FilterFunction` is currently `Blur`-only (`types.rs:~132`). The cheap
multiplicative filters (`brightness/contrast/grayscale/invert/saturate`) would be
small-render via a color tint/matrix folded into `color_tint`
(`batch.rs:~447`), but the stylesheet uses `drop-shadow`, the expensive case.

### `word-break`  — text-layout
Add `word_break: WordBreak` field + a `"word-break"` arm, then thread it into a
`buffer.set_wrap(...)` call in `shaped_buffer` (`layout.rs:~337`) and into the
measure/shape cache keys.

### `mix-blend-mode`  — renderer (blend)
Hardware blend beyond `over` needs N pre-baked pipeline variants selected per
draw span (a change to the draw-span/batching machinery in `gpu.rs`) or, for
non-separable modes, dest-texture sampling in the shader (offscreen pass, like
backdrop-blur). A parse-noop would silently not render `multiply` — a
correctness-masking trade-off; flag for review before doing it.

### `background-position` / `-size` and the `ellipse … at …` radial form
Belongs with a `BackgroundLayer { position, size, repeat }` struct + a per-layer
paint loop replacing the single bg quad (`batch.rs:~1657-1707`), box-relative
resolve at paint (`ObjectPosition` at `types.rs:~776` is the model). The
`ellipse <size> at <pos>` radial-gradient form is a separate parser extension in
the existing radial branch.

### `vertical-align`, `scroll-margin`
`vertical-align` needs an inline/baseline layout model the engine lacks (low
value). `scroll-margin` has no consumer (no scroll-snap/anchor system), so even a
field would be inert — clears a diagnostic only.

### `calc()`  — value-form
Add `parse_calc` via `parser.parse_nested_block` as a `try_parse` branch inside
the leaf parsers (`parse_px` / `parse_dimension`) so every property benefits at
once. Mixed-unit calc that must survive to layout needs a
`Dimension::Calc(Box<CalcExpr>)` variant threaded through every `*_to_taffy`
(`dim_to_taffy`, `types.rs:~1507`), `scale_dim` (`types.rs:~1733`), and
`lerp_dimension` (`transition.rs:~271`) — the biggest structural change in the
set.

### `inherit` / cascade-aware custom properties  — cascade
`inherit` (and `initial`/`unset`): intercept the keyword right after
`expect_colon` (before the property match, `parse.rs:~1172`), emit a
`StyleDeclaration::Inherit(PropertyId)` marker, and resolve it in the cascade by
copying the named field from the parent (the cascade already has the parent via
`inherit_from`; a `PropertyId` enum + per-property plumbing are missing).
Cascade-aware custom properties: today `var()` is a global parse-time text
substitution seeded only from `:root` (`extract_custom_properties` /
`resolve_var_references`, `parse.rs:~600/664`), so `.app.theme-*` token overrides
are dropped and theming uses concrete declarations. Resolving `var()` per element
during the cascade would let theme blocks override tokens — the highest-value
cascade item, but it touches the core resolution model.

## Commands

- Guardrail: `cargo test --test stylesheet_coverage`
- Core engine tests: `cargo test -p unshit-core`
- Lint (the strict form that compiles test targets):
  `cargo clippy -p unshit-core -p unshit-renderer --all-targets -- -D warnings`
- Visual smoke: `./scripts/palette-shot.ps1`

## Boundaries

- Prefer teaching the engine over parse-noops; a noop that hides a real visual
  effect (`mix-blend-mode`, `filter: drop-shadow`, real `text-shadow`) is a
  masking trade-off and must be flagged before merging.
- When an item lands, remove it from `KNOWN_UNSUPPORTED` **and** from this spec,
  and only remove a property from the inventory once it genuinely stops dropping
  (the guardrail self-polices this).
- Keep the f32 `border_radius` / `padding` fast paths intact when extending the
  unit-preserving `*_src` mirrors.

## Open questions

- Order after `transform`: is `text-overflow: ellipsis` (high value, text-layout)
  or cascade-aware custom properties (enables real theme token overrides) the
  next priority? Both are large; pick based on whether theming or list-row
  truncation is more user-visible next.
