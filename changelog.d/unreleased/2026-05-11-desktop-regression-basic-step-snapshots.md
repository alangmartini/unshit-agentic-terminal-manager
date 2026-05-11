# Desktop Regression Basic Step Snapshots

## Fixed

- Fixed `--observe basic` desktop-regression runs so suites that capture step
  snapshots now write `pre-snap` / `post-snap` diagnostic snapshots and can run
  existing cross-layer assertions without requiring `--observe full`.

## Changed

- Clarified the desktop-regression observe-mode documentation: basic includes
  diagnostic step snapshots, while full adds deterministic mode, step markers,
  invariant evaluation, and full-only checks.

## Notes

- Headed desktop suite execution remains manual because it controls the real
  Windows desktop and sends global input.
