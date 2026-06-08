### Changed

- Redesigned the sidebar workspace right-click menu to the "submenu flyout" layout: each action row now leads with an icon and (for navigational actions) a keyboard-hint badge, the shell list moved into a hover flyout that spawns to the side of "New terminal" with favourite shells starred, and the destructive actions (Kill all terminals, Remove workspace) are fenced into a grouped danger zone.

### Added

- Local automation to screenshot the sidebar context menu (open + flyout states) and score its visual parity against the design reference (`tools/sidebar-menu-shot.ps1`, `tools/menu-parity.ps1`).
