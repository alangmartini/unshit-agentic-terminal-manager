### Added

- **Alternate scroll mode (DECSET 1007).** A full-screen program that turns
  this mode on and leaves the mouse to the terminal, such as Codex on its
  pager and onboarding screens, now scrolls with the wheel: each line of
  wheel motion becomes a cursor Up or Down key. Mouse reporting still takes
  priority, the main screen keeps scrolling its own history, and
  Shift+wheel still scrolls local history. The mode survives reattaching
  to a running session.

### Fixed

- **The mouse wheel scrolls Codex again.** Wheel reports sent to a program
  that captures the mouse now name the cell under the pointer instead of
  always claiming the top-left corner. Codex ignores wheel events outside
  its transcript, and its top row is a sticky prompt header, so every notch
  used to be dropped. Programs that ignore the position, such as Claude
  Code, behave as before.
