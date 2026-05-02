# Pixel Parity Harness

This is a Windows-first local harness for comparing terminal-manager pixels
against Windows Terminal.

Run:

```powershell
pwsh tools/parity/run_parity.ps1
```

Outputs land in `artifacts/parity/latest/`:

- `windows-terminal.png`
- `terminal-manager.png`
- `diff.png`
- `report.json`
- `windows-terminal-full.png`
- `terminal-manager-full.png`
- `windows-terminal-scene.json`
- `terminal-manager-scene.json`

The harness launches terminal-manager with a unique `TM_PTYD_SOCKET` named
pipe, a parity-only shell override, and a parity-only Windows Terminal color
baseline, so it does not reuse the normal terminal-manager daemon or modify
saved user settings. Windows Terminal uses the selected/default profile, so
font, antialiasing, and profile drift still depend on that profile being kept
stable.

Useful calibration knobs:

```powershell
pwsh tools/parity/run_parity.ps1 `
  -WtProfile "PowerShell" `
  -WtColorScheme "Campbell" `
  -TerminalManagerCrop "148,236,832,216" `
  -WindowsTerminalCrop "47,120,832,216" `
  -SceneInitialDelayMs 5000 `
  -CaptureDelayMs 9000
```

To tune the two apps toward the same character grid without changing the
global window default, override either side independently:

```powershell
pwsh tools/parity/run_parity.ps1 `
  -TerminalManagerWindowWidth 1625 `
  -TerminalManagerWindowHeight 896 `
  -WindowsTerminalWindowWidth 1576 `
  -WindowsTerminalWindowHeight 816 `
  -SceneHoldSeconds 5
```

Those numbers are a local calibration point for this workspace; trust
`scene_size_comparison.match` in `report.json` and retune per display/profile.
The terminal-manager parity profile defaults to a 16px terminal font and
1.15 line height, which approximates Windows Terminal's 12pt Cascadia Mono
profile on this display. Override `TM_PARITY_FONT_SIZE_PT` or
`TM_PARITY_LINE_HEIGHT` in the parent shell to tune metrics without code
changes. The renderer also applies parity-only horizontal calibration defaults:
`TM_PARITY_CONTENT_X_OFFSET=3` and `TM_PARITY_CELL_WIDTH_SCALE=0.996`.
Override those in the parent shell to A/B render alignment without changing
normal app configuration. Set `TM_PARITY_FONT_FAMILY` to A/B a specific
terminal font family.

When testing a build from an alternate Cargo target directory, point the
harness at that binary. The matching `unshit-ptyd` sibling is discovered
automatically; use `-DaemonExe` only when the daemon lives somewhere else.

```powershell
pwsh tools/parity/run_parity.ps1 `
  -SkipBuild `
  -TerminalManagerExe target\parity-build\debug\terminal-manager.exe
```

By default the harness uses screen capture for terminal-manager, because
`PrintWindow` can return nonblank but stale or incorrectly scaled wgpu frames,
and window capture for Windows Terminal. Override with
`-TerminalManagerCaptureMode` or `-WindowsTerminalCaptureMode` if needed.
Supported capture modes are `Screen`, `Window`, `WindowStrict`, and
`PythonWindow`. `Window` keeps the historical behavior: it tries `PrintWindow`
first, then falls back to foreground screen capture. `WindowStrict` and
`PythonWindow` never use foreground screen fallback, so they can capture
unfocused or covered windows when the app provides live pixels through
`PrintWindow`.

Use background capture when you want to keep using the desktop while the
harness runs:

```powershell
pwsh tools/parity/run_parity.ps1 `
  -SkipBuild `
  -TerminalManagerExe target\parity-build\debug\terminal-manager.exe `
  -TerminalManagerWindowWidth 1625 `
  -TerminalManagerWindowHeight 896 `
  -WindowsTerminalWindowWidth 1576 `
  -BackgroundCapture `
  -SceneHoldSeconds 5
```

Background capture uses the dependency-free `tools/parity/capture_window.py`
helper and does not intentionally activate the spawned windows. Keep the
windows unminimized; Windows often stops rendering fresh GPU content for
minimized windows, so minimized capture can fail or return stale pixels. If a
background run fails, rerun without `-BackgroundCapture` and keep the spawned
Windows Terminal and terminal-manager windows visible, uncovered, unminimized,
and unmoved until the script exits. If the desktop is in use and background
capture is not viable, run only `-SelfTest` and Rust tests.

The current calibrated default crops are `TerminalManagerCrop=148,236,832,216`
and `WindowsTerminalCrop=47,120,832,216` for foreground screen captures.
When terminal-manager uses `PythonWindow`/`WindowStrict`, its calibrated
default crop is `139,212,832,216` because `PrintWindow` frames the app window
differently than screen capture.

Use the `*-full.png` captures to tune crop rectangles. Crop specs are
`x,y,width,height`. The scene waits before writing so terminal-manager can
settle its first PTY resize; the `*-scene.json` files record the console size
seen immediately before output, and `report.json` includes
`scene_size_comparison` so grid-size mismatches are obvious before pixel
metrics are interpreted.

For a quick non-window smoke test of crop and diff logic:

```powershell
pwsh tools/parity/run_parity.ps1 -SelfTest
```
