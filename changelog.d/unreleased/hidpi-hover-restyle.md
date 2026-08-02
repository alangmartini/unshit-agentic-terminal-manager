### Added

- **Slow renderer frames persist sampled, content-free stage telemetry.** `renderer-events.jsonl` records the style scope and CPU/presentation timings through a bounded background writer so future stalls can be localized without blocking rendering.

### Fixed

- **Pointer hover changes at non-default Windows display scaling no longer restyle the entire document.** Narrow pseudo-class restyles inherit logical, unscaled parent styles and avoid compounding DPI scale while preserving scoped style work.
