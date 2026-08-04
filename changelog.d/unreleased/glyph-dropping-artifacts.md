### Fixed

- **Missing-letter rendering artifacts no longer persist.** Text runs and terminal grid rows that hit a transient glyph shaping/rasterization failure are no longer stored in the cross-frame caches, so a dropped glyph is retried on the next frame instead of replaying as a permanent hole app-wide. DirectWrite rasterizations that come back 0×0 now fall through to the swash rasterizer instead of silently dropping the glyph, and shaping failures are no longer negative-cached.
- **Glyph atlas eviction runs during sustained animation.** The periodic eviction check previously only existed on the slow redraw path, so 120hz fast-path streaks let the atlas fill monotonically toward exhaustion.

### Added

- **Silent glyph drops are now observable.** Frames that drop glyphs emit a rate-limited `renderer.glyph_raster_failure` warn log and a sampled, content-free `renderer.glyph_drop` record in `renderer-events.jsonl` (failure and cache-bypass counts only), so the artifact is diagnosable from telemetry alone.
