### Added

- Fractional wheel-scroll accumulation for the terminal scrollback (scroll-smoothness spec Phase 2): wheel and touchpad deltas accumulate with sub-line precision instead of rounding every event away from zero with a 1-row minimum, so scroll distance is exactly proportional to input (no more 7-rows-per-notch over-travel or 5x touchpad amplification). Carry is discarded when the view clamps at either end of scrollback or snaps to live.
- Scroll handlers can return a `ScrollGridPatch` to update terminal grid content as a paint-only patch; wheel scrolling over the terminal no longer forces a full UI tree rebuild and no longer interrupts concurrent smooth-scroll animations.

### Changed

- Mouse-wheel notch normalization now divides unconditionally by the OS wheel setting (queried per event: `SPI_GETWHEELSCROLLLINES` vertically, `SPI_GETWHEELSCROLLCHARS` horizontally), removing the 3x amplification of sub-notch deltas from high-resolution wheels. One detent always scrolls exactly the configured scroll distance; a non-default Windows "wheel scroll lines" preference no longer multiplies it.
