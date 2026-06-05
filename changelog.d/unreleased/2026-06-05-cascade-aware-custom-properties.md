# CSS engine: cascade-aware custom properties (per-theme `var()`)

## Changed

- `var()` is no longer a global, `:root`-only parse-time text substitution.
  Token-declaring blocks (`:root`, every `.app.theme-*`, every `.theme-chip.*`)
  are collected into per-scope token maps with their **raw** values;
  `var()`-bearing declarations are captured as deferred carriers and resolved
  **per element at cascade time** against an ordered scope chain
  (`[self-widget scope, active .app.theme-* root, :root]`), unwinding
  token→token references **multi-level** against the same chain (cycle-guarded),
  then re-parsed through the existing value parsers. So theme-block `--token`
  overrides finally apply — including multi-level base-scope aliases
  (`--cp-accent: var(--amber-300)` now picks up a theme's `--amber-300`, e.g. the
  `.cp-mode-pill` prompt and the `.theme-chip` inset shadow take their theme
  color). Custom-property drops dropped from **579 → 0**.

## Notes

- Staged behind a regression oracle: `tests/cascade_golden.rs` freezes the
  resolved style of 7 themes × 8 key selectors — every clone-covered row stays
  byte-identical, only `var()`-only rows flip to their *correct* theme value —
  and asserts the custom-property drop count is `0`. Non-`var()` declarations
  keep the byte-for-byte typed fast path. A parse-time coverage pass routes any
  unresolvable scoped `var()` into `dropped`, so the `stylesheet_coverage`
  guardrail still catches a malformed token.
- Follow-ups (non-blocking, tracked in `specs/css-engine-stylesheet-gaps.md`):
  the self-scope walk gate is conservative on this stylesheet (a cleaner gate
  builds the scope env only for elements that actually have a deferred
  declaration), and the ~690 hand-authored per-theme concrete declarations that
  previously faked theming can now be retired one theme at a time.
