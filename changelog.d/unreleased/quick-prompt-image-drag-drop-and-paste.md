### Added

- The Quick Prompt overlay can now attach images two new ways, in addition to the existing paste pipeline:
  - **Drag-and-drop** — drop one or more image files (PNG/JPEG) onto the window to attach them. Non-image drops (folders, text files, unsupported formats) are skipped, and a hint is shown when a drop contained no usable image.
  - **Clipboard paste** — press **Ctrl+V** while the overlay is open to attach an image from the clipboard. A paste with no image on the clipboard is a silent no-op.
- Both paths reuse the existing pasted-image handling: full-resolution PNG plus thumbnail, content-addressed so duplicates are de-duplicated, with identical chips, submit, and cleanup behavior.
