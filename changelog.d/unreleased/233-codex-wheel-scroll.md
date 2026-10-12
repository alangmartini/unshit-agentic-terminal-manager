### Fixed

- **The mouse wheel scrolls Codex again.** Wheel reports sent to a program
  that captures the mouse now name the cell under the pointer instead of
  always claiming the top-left corner. Codex ignores wheel events outside
  its transcript, and its top row is a sticky prompt header, so every notch
  used to be dropped. Programs that ignore the position, such as Claude
  Code, behave as before.
