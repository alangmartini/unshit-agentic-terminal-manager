### Added

- **Pane and tab names follow the program's window title (OSC 0/2).** Like Windows Terminal, a pane's label — in the sidebar, the tab bar, and the tab context menu — now updates live when the running program sets its terminal title, so Claude Code, Codex, ssh, and title-setting shells name their own tabs (e.g. `✳ claude`). Manual renames still win: a pane you renamed keeps its name (also across restarts) until you clear the rename, which hands control back to the program. Titles are sanitized for display (control characters stripped, length capped), a bare executable-path title collapses to the program name (`powershell` instead of `C:\...\powershell.exe`), and an empty title falls back to the generic `shell` label.

### Fixed

- **Sidebar terminal names no longer collapse to `…` next to a long git-branch chip.** The row now lets the branch chip shrink (down to a small floor) before the terminal name loses width, so longer names — including the new program-set titles — stay readable at the default sidebar width.
