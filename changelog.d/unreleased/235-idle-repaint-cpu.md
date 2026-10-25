### Fixed

- **The app no longer burns CPU while nothing is happening.** On Windows the
  window repainted at the display refresh rate for a quarter second after
  every background event, and the terminal cursor blink plus the once-a-second
  resource stats were enough to keep it painting identical frames most of the
  time: about 0.14 of a core with idle shells, and 0.3 to 0.4 while agent
  panes animated. Heartbeats with nothing new to show are now skipped, and the
  cursor blink stops requesting repaints while the window is unfocused. Idle
  CPU drops to about 0.04 of a core and a busy agent spinner to about 0.07.
  Text-input carets and CSS animations also keep animating when the window is
  otherwise quiet instead of freezing until the next event, and the endless
  attention pulse on a pane that needs input runs at about 25 frames per
  second instead of the display rate.

### Added

- **Frame rate telemetry.** `renderer-events.jsonl` now carries a
  `renderer.frame_summary` record per minute with the number of painted
  frames, rebuilds, summed stage work and skipped heartbeats, so idle CPU
  can be diagnosed from the log alone. `scripts/idle-cpu-bench.ps1`
  reproduces the idle and busy-agent measurements on an isolated instance.
