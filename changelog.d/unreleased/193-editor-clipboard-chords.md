### Fixed

- **Paste and copy in editor panes.** `Ctrl+V`, `Ctrl+Shift+V` and
  `Shift+Insert` are registered application bindings, so the shortcut
  resolver claimed them before the focused editor pane could see the key:
  the paste looked for a terminal, found none, and every attempt ended in a
  `paste failed: no terminal in focus` toast with the clipboard never
  reaching the document. `Ctrl+Shift+C` copied nothing for the same reason.
  Both commands now recognise an editor pane and act on its buffer — paste
  lands as one undo step and keeps multi-line clipboard payloads on
  separate lines (terminal paste promotes newlines to carriage returns for
  the shell, which would otherwise collapse them), copy takes the buffer
  selection. Pastes are recorded as `editor.paste` (path and counts only)
  in the profile's `editor-events.jsonl`.
