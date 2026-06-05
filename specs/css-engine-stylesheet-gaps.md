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
- **Tier 2 — `text-overflow: ellipsis` landed 2026-06-05** (the top tier-2
  candidate): grapheme-cluster-correct truncation that measures the *painted*
  composed (`prefix + …`) width over *logical* prefixes, so the fit holds for
  LTR / RTL / bidi / combining-mark text at any letter-spacing. See
  `changelog.d/unreleased/2026-06-05-text-overflow-ellipsis.md`.
- **Tier 2 — cascade-aware custom properties landed 2026-06-05** (the largest
  drop category, **579 → 0**): `var()` now resolves *per element* against the
  active scope chain (`[self-widget scope, active .app.theme-* root, :root]`)
  instead of a global parse-time `:root`-only textual pass, so theme-block
  `--token` overrides apply — including multi-level base-scope aliases
  (`--cp-accent: var(--amber-300)` picks up a theme's `--amber-300`). See
  `changelog.d/unreleased/2026-06-05-cascade-aware-custom-properties.md`.
- **Tier 2 — `transform` (full affine) landed 2026-06-05** (the highest-value
  remaining gap): `scale` / `rotate` / `translateX` / `translateY` and the
  combined `translateY(..) scale(..)` now compose into a per-element 2x3 affine
  about the box center, baked into every emitted instance and propagated to the
  subtree; both transitions and keyframes (modal-in `translateY+scale`, cd-lift)
  interpolate it component-wise. The renderer separates `local_pos` (in-quad
  border-radius / gradient, left untransformed) from `pixel_pos` (transformed,
  drives NDC + ancestor clip), so the fragment shaders are unchanged. See
  `changelog.d/unreleased/2026-06-05-css-transform-affine.md`.
- **Tier 2 — still deferred:** the table below. Each is genuinely renderer-,
  text-layout-, or value-evaluator-bound — not a one-line parse arm.

## Enforcement coverage (what the guardrail does / does not catch)

- **Caught (build fails on a new gap):** every per-property entry in
  `KNOWN_UNSUPPORTED`, plus any new `calc()` value (`is_known_gap` matches
  `value.contains("calc(")`) and `inherit` (`value == "inherit"`).
- **Now enforced (was the cascade gap):** `.app.theme-*` / `.theme-chip.*`
  `--token` overrides resolve through the cascade and no longer drop.
  `cascade_golden`'s `custom_property_drop_count_is_frozen` asserts the count
  stays `0`, and a scoped `var()` that cannot resolve is surfaced into `dropped`
  by a parse-time coverage pass, so the guardrail catches it.

## Deferred inventory

| Property / form | Drops today | Class | Effort | Value |
|---|---|---|---|---|
| `text-shadow` (non-`none`) | `none` accepted; real shadow lists drop | renderer (offscreen blur) | L | M |
| `filter: drop-shadow(...)` | no element `filter` field; only `backdrop-filter` blur exists | renderer (offscreen) | L | M |
| `word-break: break-word` | no `set_wrap` control in the shaper | text-layout | M | M |
| `mix-blend-mode: multiply` | blend is baked per-pipeline, not per-instance | renderer (blend) | L | L |
| `vertical-align: text-bottom` | no inline/baseline layout model | text-layout | L | L |
| `background-position` / `background-size` | single `Background` field; gradients fill the box | small-render | M | L |
| `background` (`ellipse <size> at <pos>`) | radial-gradient parser rejects this form | parse (gradient) | M | L |
| `scroll-margin` | no scroll-snap / anchor consumer | parse (inert) | S | L |
| `calc()` (value form) | no evaluator in leaf parsers | value-form | L | M |
| `inherit` (value form) | no per-property cascade keyword | value-form (cascade) | M | M |

## Plans (highest value first)

### `transform` (full affine)  — LANDED 2026-06-05
`scale` / `rotate` / `translateX` / `translateY` (+ the combined
`translateY(..) scale(..)`) compose into a per-element 2x3 affine about the box
center (transform-origin defaults to `50% 50%`; none is authored). The matrix is
threaded down the paint recursion (`parent_xform`), composed per node, and baked
into each emitted instance via a delta-from-identity `xform: [f32;4]` +
`xform_translate: [f32;2]` (zero = identity, so untransformed elements stay on
the matrix-free fast path). The original triage feared a clip-rect + gradient
rework, but the shader already separates `local_pos` (in-quad math, untransformed
— rotates/scales *with* the quad) from `pixel_pos` (transformed → NDC + the
axis-aligned ancestor clip), so **the fragment shaders are unchanged**; only the
four vertex shaders transform `pixel_pos`. The cache signature gained the affine
so an ancestor transform change re-emits descendants. The old scalar
`transform_dx` render-offset mechanism was retired. **Accepted limitation:**
descendant `overflow` clipping is computed in the element's untransformed space,
so a *rotated* clipping ancestor may mis-clip mid-animation (exact at rest, since
animated transforms rest at identity). See
`changelog.d/unreleased/2026-06-05-css-transform-affine.md`.

### `text-overflow: ellipsis`  — LANDED 2026-06-05
`TextOverflow {Clip, Ellipsis}` field + non-inheriting parse arm; the render gate
(`Ellipsis && white-space:nowrap && overflow`) calls
`layout::truncate_text_with_ellipsis`, which iterates **logical** cluster
boundaries and keeps the largest prefix whose **painted** composed width
(`painted_run_width`, the renderer's exact `glyph.x + idx*letter_spacing + glyph.w`)
fits `content_w`. This is correct for LTR/RTL/bidi/combining + any letter-spacing
(the original visual-order walk overflowed bidi by up to ~5×). Accepted limitation:
RTL truncates the logical tail rather than the CSS-perfect visual-left end (fit is
always guaranteed). Sub-pixel atlas-bitmap overhang vs the advance-based fit
formula is a universal, pre-existing concern covered by a 0.5px epsilon.

### `text-shadow` (real, non-`none`)  — renderer (offscreen blur)
`none` is already accepted (tier 1). **Correction (was mis-scoped as a cheap
"sharp re-emit"):** every `text-shadow` the app actually authors is a *zero-offset
blurred glow* — `0 0 8px …` (workspace name), `0 0 6px var(--accent-a35)` (the
prompt), `0 0 8px …` (search highlight). A sharp re-emit at offset `(0,0)` with no
blur would draw the shadow glyphs exactly behind the main glyphs (fully occluded)
and render *nothing* of the intended halo — i.e. a masking no-op the Boundaries
below forbid. So for this stylesheet `text-shadow` is the **large-render** path:
an offscreen alpha pass of the glyph run + a separable blur + composite under the
text (the same machinery `filter: drop-shadow` needs). A `text_shadow` field +
`parse_text_shadow_list` (modeled on `parse_box_shadow_list`, `parse.rs:~3229`)
is the easy part; the blur pass is the work. Sharp/offset shadows (none authored
here) would be the cheap re-emit, but are not what unblocks the app.

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

### `inherit` (and `initial` / `unset`)  — cascade
Intercept the keyword right after `expect_colon` (before the property match,
`parse.rs`), emit a `StyleDeclaration::Inherit(PropertyId)` marker, and resolve it
in the cascade by copying the named field from the parent (the cascade has the
parent via `inherit_from`; a `PropertyId` enum + per-property plumbing are
missing).

### cascade-aware custom properties  — LANDED 2026-06-05
`var()` is no longer a global `:root`-only parse-time substitution. Token-declaring
blocks are collected into per-scope `TokenScopes` (raw values, **not**
pre-flattened); `var()`-bearing declarations are captured as
`StyleDeclaration::Deferred` carriers and resolved per element at cascade time
against an ordered `ScopeEnv` (`[self-widget scope, active .app.theme-* root,
:root]`), unwinding token→token references multi-level against the same env
(cycle-guarded), then re-parsed through the existing leaf parsers. Theme-block
overrides now apply (drops 579 → 0); a parse-time coverage pass routes
unresolvable scoped `var()` into `dropped` for the guardrail.
Follow-ups (non-blocking): **(a) perf** — *corrected:* the self-scope walk gate
is **already live** on this stylesheet, not defeated. Every non-base token scope
(`.app.theme-*`, `.theme-chip.<name>`) has a class-bearing terminal compound, so
`widget_scope_gate_unsafe` is `false` (asserted by `parse.rs`'s
`widget_scope_classes_gate_skips_non_widget_elements` test) and
`element_may_have_self_scope` returns `false` for the overwhelming majority of
elements, skipping the walk. The earlier claim that "every element pays the scope
walk" was wrong; the cleaner Deferred-gated `ScopeEnv` build is a marginal tidy-up,
not a fix. **(b)** the `var(` capture gate doesn't catch `var()` immediately after
`)`. **(c)** *corrected:* the ~690 "hand-authored concrete clones" are a
pre-Stage-3 artifact already retired — only ~16 concrete per-theme declarations
remain and none are redundant (the cascade golden confirms current output is
correct), so there is effectively no clone-retirement work left.

## Commands

- Guardrail: `cargo test --test stylesheet_coverage`
- Cascade golden + var resolution: `cargo test --test cascade_golden`,
  `cargo test -p unshit-core --test token_scopes`,
  `cargo test -p unshit-test --test scoped_var_resolution`
- Core engine tests: `cargo test -p unshit-core`
- UI / interaction tests (**MUST run** — overflow/scroll regressions hide here and
  are invisible to the other gates): `cargo test -p unshit-test`
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

- `text-overflow: ellipsis`, cascade-aware custom properties, and the full
  `transform` affine are now landed — the three highest-value gaps. What remains
  in the inventory is genuinely lower-value or genuinely large:
  - **`word-break: break-word`** (M/M) is the only remaining *tractable* gap with
    real (if narrow, single-use) value — one `.term-line` use; the trap is the
    shape/measure cache key (a missed field silently serves stale shaping).
  - **`text-shadow`** is NOT a cheap win for this app (every authored value is a
    zero-offset blur glow → offscreen-blur path; see its plan). Same bucket as
    `filter: drop-shadow`.
  - The remaining table entries are all L-value (`mix-blend-mode`,
    `vertical-align`, `background-position/size`, `scroll-margin`) or large
    value-form work (`calc()`, `inherit`).
- The two cascade "follow-ups" both evaporated under inspection (see the
  cascade-aware section): the perf gate is already live, and the clone retirement
  is already done. Neither is pending work.
