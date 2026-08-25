### Added

- **The mouse wheel now scrolls the pane under the pointer.** In a split, hovering the other half and scrolling moves *that* pane's scrollback instead of doing nothing — reading back through a background pane no longer costs a click to focus it first, and scrolling deliberately leaves keyboard focus where it is. Applies to terminal and editor panes alike; a pane running a mouse-tracking TUI receives the wheel as mouse reports the same way the focused pane does. Keyboard input is unchanged and still goes only to the focused pane.
