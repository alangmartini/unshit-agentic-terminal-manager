### Added

- Mouse text selection in the terminal: click-drag to select, double-click to select a word (path-aware), triple-click to select a line, and Shift+click to extend. Selected cells are highlighted on the per-frame render clone without touching the live buffer.
- Copy support following Windows Terminal conventions: `Ctrl+C` copies when there is a selection (and still sends `SIGINT` when there is none), `Ctrl+Shift+C` always copies. Copying clears the selection.
- Right-click pastes the clipboard into the pane (classic Windows console behavior); `Shift+Insert` also pastes.
- Bracketed paste: the terminal now tracks DECSET 2004 and `terminal.paste` wraps pasted text in `ESC[200~`/`ESC[201~` when the running program enabled it, so shells and editors can distinguish a paste from typed input.

### Changed

- The UI framework's `DragEvent` and `MouseEvent` now carry element-local pointer coordinates (`local_x`/`local_y`), and `MouseDown` is dispatched to element handlers, so grid/canvas widgets can map a pointer to a cell without re-deriving their absolute rect. Added a `Key::Insert` variant and its winit mapping.
