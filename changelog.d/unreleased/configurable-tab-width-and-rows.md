### Added

- Configurable horizontal tab strip in Settings → Appearance → **tabs**:
  - **Tab sizing** — `fixed` pins every tab to a configurable width, or `fit content` shrink-wraps each tab to its own label.
  - **Tab width** — a stepper (120–400px, default 200px) for the fixed width; hidden in fit-content mode where there is nothing to tune.
  - **Tab rows** — keep the historical `single` scrolling row, or wrap the strip onto `double`/`triple` stacked rows. In multi-row mode the tab bar grows downward (the terminal grid below shrinks) and the `>`-style horizontal overflow is dropped; once tabs exceed the row cap the strip scrolls vertically instead.

### Changed

- Tabs now default to a fixed 200px width (previously a 150–240px content-clamped band). Width, sizing mode, and row mode are all adjustable from the appearance settings and reset with the rest of the appearance section.
