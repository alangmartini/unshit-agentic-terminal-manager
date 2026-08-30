### Added

- Pasting into a terminal pane now handles copied *files*, not just copied
  text or bitmaps. ShareX's "copy file to clipboard" after-capture action,
  or plain Ctrl+C on files in Explorer, puts a `CF_HDROP` file list on the
  clipboard with no text at all — previously Ctrl+V silently did nothing
  with it. The paste now inserts each file's path, quoted when it contains
  spaces and space-separated for multi-file copies, so agent CLIs (Claude
  Code, Codex) attach the image exactly like a drag-and-drop and a plain
  shell receives usable path arguments. Precedence is text, then file list,
  then bitmap: when a file list and a bitmap are both present, the on-disk
  file is the original bytes, so its path wins over re-encoding pixels to a
  temp PNG.

- The Quick Prompt learned the same trick: Ctrl+V (and the "Attach image"
  button) with copied image files on the clipboard attaches every decodable
  image among them as chips, exactly like dropping the files on the overlay.
  Copied non-image files still fall through to the normal text paste
  silently.
