### Changed

- **`Ctrl+=` / `Ctrl+-` now zoom the whole interface, not just the terminal.** Zoom scales the DPI factor that every computed style is resolved against, so spacing, borders, icons, the sidebar, the tab strip and the terminal grid all grow and shrink together. The terminal reflows its PTY to the new cell metrics as part of the change. `Ctrl+0` resets to 100%, and Settings > Appearance shows the current level. The separate terminal and config font-size steppers in Settings are unchanged.

### Fixed

- **`Ctrl` + mouse wheel zoom now has a visible effect.** The zoom factor it maintained was never folded into style scaling, so the gesture only cleared caches and forced a rebuild at the old size.
