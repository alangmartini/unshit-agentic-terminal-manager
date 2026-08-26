### Fixed

- **Reattached shells no longer keep the previous window's size.** Sessions outlive the app, but nothing told a surviving session how big its pane had become: reattaching carried no dimensions at all, and reusing a session ignored the ones it was sent, so a shell spawned in a maximized window stayed that tall in a smaller one. A resize that arrived before its pane had a session was dropped outright with no retry, and since a pane only reports its size when its rectangle changes, the shell then kept the wrong geometry for the rest of the run. Full-screen programs — an agent CLI, `vim`, anything that redraws a frame — drew for rows the pane no longer had, so their output piled onto the last visible line over stale text and the bottom of the frame was never reachable. Panes now remember the last size they asked for and replay it the moment a session appears, and reusing a live session adopts the reattaching window's geometry.

### Added

- **`pty.resize` telemetry.** Every pane geometry change now records whether it reached the shell (`applied`, `replayed`) or not (`dropped_unmapped`, `dropped_disconnected`, `rpc_failed`) to `renderer-events.jsonl`, with the pane and session ids. Resize failures used to be discarded silently, which made a mismatched shell size invisible until something drew off the bottom of the pane.
