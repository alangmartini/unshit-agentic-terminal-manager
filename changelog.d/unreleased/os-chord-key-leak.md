### Fixed

- **Window-switching chords no longer type into the shell.** Keys that were physically held when the window lost or regained focus were being replayed as real keystrokes, so the tail of an `Alt+Tab` or `Win+D` arrived at the PTY as a bare character — enough to submit a half-written prompt to an agent CLI. Those focus-sync notifications are now ignored, and the chords the window manager owns (`Alt+Tab`, `Alt+Shift+Tab`, `Alt+Space`, and every `Win` combination) no longer encode to anything the terminal can send.
