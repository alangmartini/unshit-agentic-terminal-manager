# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- **Context menus no longer open off the bottom (or right) of the window.**
  Right-clicking a workspace low in the sidebar placed the menu straight at
  the cursor, so its danger zone -- "Kill all terminals" and "Remove
  workspace" -- fell past the window edge, where it could not be clicked and
  did not scroll. Menus are now pulled back inside the window before they are
  drawn, and a `ui.ctx_menu_open` event records the anchor and window box so
  a mis-placed menu can be diagnosed from telemetry.

## [0.3.3] - 2026-08-30

Startup got an overhaul. The window now appears already drawn and answering
input while the GPU is still coming up, the longest stretch of startup happens
underneath the rest instead of in front of it, and launching no longer flashes
a train of black console windows. Paste also learned about copied files:
ShareX's "copy file to clipboard" and Ctrl+C on files in Explorer now paste
into a terminal as quoted paths and attach to the Quick Prompt as images.

### Added

- **The window now appears and responds while the GPU is still starting**,
  instead of waiting for it. GPU adapter and device creation costs over a
  second on a cold start and none of it can be skipped, but everything needed
  to know *what* to draw -- the stylesheet, the fonts, the element tree, the
  layout -- is ready long before that. The window is painted from the real,
  already-laid-out tree using the platform's own 2D drawing, then swapped for
  the GPU surface once it is ready. The placeholder is the app's own geometry
  in the app's own colors, not a generic spinner: every element that has a
  background contributes a rectangle, clipped and composited exactly as the
  layout says. It has no gradients, rounded corners, shadows or terminal
  cells, so it reads as the application mid-assembly rather than as a finished
  frame -- which is the honest thing for it to look like.
- **Pasting into a terminal pane now handles copied *files*, not just copied
  text or bitmaps.** ShareX's "copy file to clipboard" after-capture action,
  or plain Ctrl+C on files in Explorer, puts a `CF_HDROP` file list on the
  clipboard with no text at all -- previously Ctrl+V silently did nothing
  with it. The paste now inserts each file's path, quoted when it contains
  spaces and space-separated for multi-file copies, so agent CLIs (Claude
  Code, Codex) attach the image exactly like a drag-and-drop and a plain
  shell receives usable path arguments. Precedence is text, then file list,
  then bitmap: when a file list and a bitmap are both present, the on-disk
  file is the original bytes, so its path wins over re-encoding pixels to a
  temp PNG.
- **The Quick Prompt learned the same trick:** Ctrl+V (and the "Attach image"
  button) with copied image files on the clipboard attaches every decodable
  image among them as chips, exactly like dropping the files on the overlay.
  Copied non-image files still fall through to the normal text paste
  silently.

### Changed

- **GPU bring-up now starts at process entry instead of after the window
  exists**, on Windows. Adapter and device creation need no window --
  verified by measurement, an adapter requested with no compatible surface
  costs the same as one requested with it -- but they are the single longest
  stretch of startup, and they run on the event-loop thread. Starting them
  first means config load, state seeding, the daemon handshake, the event
  loop and the window all happen alongside that wait rather than in front of
  it. Measured over five cold starts, 160-480ms of GPU bring-up now happens
  underneath other startup work, median about 300ms. The prewarmed adapter is
  only used if it can actually present to the window that was ultimately
  created; on a machine where it cannot -- a second GPU driving the display
  the window landed on -- it is discarded and the original path runs
  unchanged. Backend selection goes through the same resolution the real
  request uses, environment overrides included, so `UNSHIT_RENDER_BACKEND`
  still decides and the compositor-clock D3D12/Mailbox pacing is unaffected.
- **The window now waits until it has a drawn frame behind it before
  appearing**, instead of appearing empty and then freezing. GPU adapter and
  device creation run on the event-loop thread, so a window mapped at
  creation time cannot answer a paint or a click until they finish --
  measured at about 1.2 seconds on a machine whose D3D12 adapter enumeration
  is slow. What that bought was not an early UI but a white, non-responding
  rectangle. The app now appears already drawn.
- **Work that does not have to finish before the first frame no longer holds
  it up.** Time to that first frame dropped by roughly half a second on a
  7-workspace, 10-pane profile:
  - Git branch names resolve on a background thread and appear a moment after
    the sidebar does. Workspaces sharing a repository are probed once, not
    once each. A branch that has not resolved yet renders muted rather than
    as the red "no git" error it used to flash on every launch.
  - Panes that are not visible on the first frame reattach to the daemon in
    the background, taking the state lock one pane at a time so a long
    restore cannot stall the UI. The active pane still comes up eagerly,
    exactly as before.
  - Terminal cell metrics are measured against the embedded JetBrains Mono
    face instead of building a font database from every font installed on the
    machine. This is also a correctness fix: the old measurement asked the OS
    for `monospace` and got Consolas, whose advance width the renderer never
    uses.

### Fixed

- **Starting the app no longer flashes a series of black console windows on
  screen before the UI appears.** The release binary owns no console, so
  every `git` subprocess it spawned made Windows allocate a fresh console
  window for the life of the child -- once per restored workspace. All `git`
  invocations now run with `CREATE_NO_WINDOW`, and a test rejects any new
  call site that does not.
- **Launching the app no longer waits a fixed 25ms before its first attempt
  to reach the `unshit-ptyd` daemon**, and no longer retries at all when the
  daemon socket does not exist yet. The pause was paid on every single
  launch, including the common case where the daemon was already running and
  answered immediately. Connecting now starts at 0.5ms of backoff and only
  retries when the endpoint exists but is momentarily busy -- which is the
  one case a retry can actually help.
- **A background pane whose reattach failed now still refreshes the UI**, so
  the spawn failure it recorded becomes visible instead of sitting in state
  until something else happens to trigger a rebuild.

## [0.3.2] - 2026-08-26

A pane's shell now always knows how big its pane actually is. Reattaching to a
session that outlived the app, resizing a split, or resizing the window used to
leave shells wrapping at a stale width or drawing for rows the pane no longer
had — an agent CLI or any full-screen program would pile its frame onto the last
visible line over stale text. The mouse wheel also follows the pointer now, and
window-switching chords no longer leak keystrokes into the shell.

### Added

- **The mouse wheel now scrolls the pane under the pointer.** In a split, hovering the other half and scrolling moves *that* pane's scrollback instead of doing nothing — reading back through a background pane no longer costs a click to focus it first, and scrolling deliberately leaves keyboard focus where it is. Applies to terminal and editor panes alike; a pane running a mouse-tracking TUI receives the wheel as mouse reports the same way the focused pane does. Keyboard input is unchanged and still goes only to the focused pane.
- **`pty.resize` telemetry.** Every pane geometry change now records whether it reached the shell (`applied`, `replayed`) or not (`dropped_unmapped`, `dropped_disconnected`, `rpc_failed`) to `renderer-events.jsonl`, with the pane and session ids. Resize failures used to be discarded silently, which made a mismatched shell size invisible until something drew off the bottom of the pane.

### Fixed

- **Reattached shells no longer keep the previous window's size.** Sessions outlive the app, but nothing told a surviving session how big its pane had become: reattaching carried no dimensions at all, and reusing a session ignored the ones it was sent, so a shell spawned in a maximized window stayed that tall in a smaller one. A resize that arrived before its pane had a session was dropped outright with no retry, and since a pane only reports its size when its rectangle changes, the shell then kept the wrong geometry for the rest of the run. Full-screen programs — an agent CLI, `vim`, anything that redraws a frame — drew for rows the pane no longer had, so their output piled onto the last visible line over stale text and the bottom of the frame was never reachable. Panes now remember the last size they asked for and replay it the moment a session appears, and reusing a live session adopts the reattaching window's geometry.
- **Background panes in a split no longer keep the wrong size after a resize.** Only the focused pane told its shell how big it had become, so resizing the window, dragging a splitter or collapsing the sidebar left every other pane's shell still wrapping at the old width — a `top`, a build log or an agent CLI in the other half of a split would paint at the stale geometry until you clicked it, which was the only thing that corrected it. Every visible pane now reports its own size, and a pane whose size has not actually been measured yet is skipped instead of pushing a one-column geometry at a live shell.
- **Window-switching chords no longer type into the shell.** Keys that were physically held when the window lost or regained focus were being replayed as real keystrokes, so the tail of an `Alt+Tab` or `Win+D` arrived at the PTY as a bare character — enough to submit a half-written prompt to an agent CLI. Those focus-sync notifications are now ignored, and the chords the window manager owns (`Alt+Tab`, `Alt+Shift+Tab`, `Alt+Space`, and every `Win` combination) no longer encode to anything the terminal can send.

## [0.3.1] - 2026-08-24

Interface zoom now scales the whole app, and the tab strip scrolls with a plain
vertical wheel. If you are still on v0.2.6 or earlier, upgrade: those builds leak
a Win32 event handle per presented frame and grow by roughly 600 MB of memory per
day (see the 0.3.0 note below). The leak was already fixed in 0.3.0.

### Changed

- **`Ctrl+=` / `Ctrl+-` now zoom the whole interface, not just the terminal.** Zoom scales the DPI factor that every computed style is resolved against, so spacing, borders, icons, the sidebar, the tab strip and the terminal grid all grow and shrink together. The terminal reflows its PTY to the new cell metrics as part of the change. `Ctrl+0` resets to 100%, and Settings > Appearance shows the current level. The separate terminal and config font-size steppers in Settings are unchanged.

### Fixed

- **`Ctrl` + mouse wheel zoom now has a visible effect.** The zoom factor it maintained was never folded into style scaling, so the gesture only cleared caches and forced a rebuild at the old size.
- **The mouse wheel now scrolls the tab strip.** A vertical wheel over a container that only overflows horizontally is translated into horizontal movement, the way browsers do, so a mouse without a tilt wheel can reach tabs that have scrolled off the strip. Containers that can scroll vertically, and trackpads that already report a horizontal delta, are untouched.

## [0.3.0] - 2026-08-20

Agentic workflow release: Claude Code and Codex conversations survive a machine
restart, new tabs can open on their own fresh git worktree, a built-in file
editor lands, tabs name themselves from the running program's title, and the
Windows renderer holds a real 120 Hz budget without dropping glyphs.

### Added

- **Claude Code and Codex conversations are restored after the PTY daemon is
  lost, including a machine restart.** When Terminal Manager is next opened,
  saved agent panes with an exact or unambiguous conversation id show a
  provider-specific manual Resume chip by default, with an opt-in automatic mode
  under Settings > Sessions. This feature does not register Windows login
  startup.
- **Exact conversation ids are captured through consent-gated, idempotent Claude
  Code and Codex `SessionStart` hooks.** Enabling automatic recovery installs the
  managed hooks immediately; disabling it leaves them available for manual
  recovery, and a separate Remove recovery hooks control removes only Terminal
  Manager entries.
- **Workspace ids are now stable numeric values**, so deleting one workspace
  cannot renumber another workspace's daemon session keys.
- **Built-in file editor (MVP).** Open a text file in an editor pane via the
  `editor.open:<path>` command. The editor renders through the terminal's GPU
  cell-grid pipeline (viewport-only publication, stable line identities for
  cached line replay), shows a line-number gutter, and scrolls by keyboard
  (arrows, PageUp/PageDown, Ctrl+Home/End) and mouse wheel. Oversized (>16 MiB)
  and non-UTF-8 files are refused with a notification instead of being lossily
  converted. Full editing is supported: typing (including dead-key composition),
  selection by keyboard and mouse (click, drag, double-click word,
  triple-click/gutter line), clipboard (Ctrl+C/X/V), undo/redo with VS Code-style
  grouping, and atomic save (Ctrl+S, sibling temp file + rename) that preserves
  the file's CRLF/LF line endings (stray bare-CR line endings in classic-Mac or
  mixed files are normalized to line breaks on load, like VS Code).
  `Ctrl+Shift+O` (or the palette's "Open file...") opens a native file picker;
  Shift+wheel scrolls horizontally. Closing a pane or tab with unsaved changes
  prompts save / discard / cancel. Editor panes are session-local and are not
  persisted across restarts. Editor lifecycle telemetry (open/save/close, never
  file content) is recorded to `editor-events.jsonl`.
- **New tabs can open on a fresh git worktree.** The command palette
  (`Ctrl+Shift+P`) gains two commands: **New worktree tab** opens a new tab whose
  shell starts inside a freshly created `git worktree` of the active workspace's
  repo (on its own `godly-wt-<hex>` branch, so work done there is easy to merge
  back), and **Toggle worktree tabs** turns on a mode where every new tab —
  `Ctrl+T`, the tab-bar `+`, or a workspace's "new terminal" — opens on its own
  fresh worktree. The mode shows an `on` pill in the palette while active and
  persists across restarts. Worktrees are created under the app's per-profile
  `worktrees` directory (the same base Quick Prompt uses). Non-repo workspaces
  fall back to a plain tab in mode, and the one-shot command explains via toast
  when the workspace has no git repo.
- **Pane and tab names follow the program's window title (OSC 0/2).** Like
  Windows Terminal, a pane's label — in the sidebar, the tab bar, and the tab
  context menu — now updates live when the running program sets its terminal
  title, so Claude Code, Codex, ssh, and title-setting shells name their own tabs
  (e.g. `✳ claude`). Manual renames still win: a pane you renamed keeps its name
  (also across restarts) until you clear the rename, which hands control back to
  the program. Titles are sanitized for display (control characters stripped,
  length capped), a bare executable-path title collapses to the program name
  (`powershell` instead of `C:\...\powershell.exe`), and an empty title falls
  back to the generic `shell` label.
- **A default-off "Start at Windows sign-in" control under Settings >
  Sessions.** Combined with the separate Automatic agent resume opt-in, Terminal
  Manager can reopen saved Claude Code and Codex conversations after a PC restart
  without waiting for the app to be launched manually.
- **A repeatable real-input typing performance gate is available through
  `cargo xtask typing-perf`.** It drives deterministic human-rate and stress-rate
  Win32 keyboard input against an isolated release build, captures input latency
  and frame cadence, and fails unless every run sustains the 120 Hz budget
  without dropped presentation slots.
- **Frame diagnostics now expose renderer work quantiles and the active display
  period.** Desktop performance tests can distinguish an overloaded renderer from
  a display refresh-rate limit while exercising real keyboard input.
- **Presentation telemetry now separates swapchain acquisition, intentional phase
  holding, and the platform present call.** Performance reports expose
  p50/p95/p99/max values for each wait source so driver pacing cannot masquerade
  as renderer work.
- **Cadence telemetry now separates compositor heartbeats from paint
  completion.** Native vblank-aligned intervals drive cadence acceptance on the
  Windows Mailbox path, while completion jitter stays independently queryable and
  every measured frame must return from its present call within one display
  period of the heartbeat.
- **Slow renderer frames persist sampled, content-free stage telemetry.**
  `renderer-events.jsonl` records the style scope and CPU/presentation timings
  through a bounded background writer so future stalls can be localized without
  blocking rendering.
- **Silent glyph drops are now observable.** Frames that drop glyphs emit a
  rate-limited `renderer.glyph_raster_failure` warn log and a sampled,
  content-free `renderer.glyph_drop` record in `renderer-events.jsonl` (failure
  and cache-bypass counts only), so the artifact is diagnosable from telemetry
  alone.

### Changed

- Restored panes are reconciled with a daemon-atomic attach-or-spawn request,
  preventing duplicate agent launches when clients race, the initial session-list
  cache is unavailable, or an IPC response is lost.
- Hook observations are authenticated with per-PTY capabilities and acknowledged
  only after the recovery record has been durably saved, with bounded
  negative-acknowledgement retries.
- Local IPC peer ownership is verified before either side accepts recovery
  traffic, and linked/reparse-point hook configuration files are refused so
  another local account or a filesystem redirect cannot capture a capability or
  overwrite an unintended file.
- Minimal routing and launch metadata is written through owner-private, synced
  atomic replacement, discovery inspects only a bounded JSONL prefix, and
  recovery events are size-bounded and redacted. Persistence and telemetry
  exclude prompts, transcript content, terminal output, hook payloads,
  conversation ids, paths, and raw errors as applicable.
- Application exit is vetoed when the final keep-running or kill-all recovery
  state cannot be saved. The updated live state remains open with an actionable
  error, preventing stale disk metadata from resurrecting an intentionally
  stopped agent.
- Login startup is registered directly in the current user's Windows `Run` key
  with profile-isolated values, bounded quoted executable paths, redacted durable
  telemetry, failure-safe UI state, and uninstall cleanup for the installed app
  value.
- **Windows now pairs D3D12's tear-free single-frame Mailbox queue with the
  native DirectComposition heartbeat, 4x MSAA, and a two-frame latency request.**
  The wgpu 30 renderer keeps the full decorative quad shader, DirectWrite text
  rasterization, gradients, masks, shadows, borders, and transforms while
  avoiding the rare double-refresh blocking acquire observed on FIFO drivers;
  unsupported systems retain the ordinary backend and tear-free FIFO fallback.
- **The Windows UI/render and compositor-clock threads register with the
  Multimedia Class Scheduler `Games` task.** This reduces scheduler preemption
  during active 120 Hz rendering while retaining safe fallbacks and structured
  lifecycle telemetry.

### Fixed

- **The UI process no longer leaks a Win32 event handle on every presented
  frame.** Builds up to and including 0.2.6 accumulated roughly 30-60 kernel
  `Event` handles per second while the window rendered, which showed up as the UI
  process growing by about 600 MB of memory per day (2 GB and 2.7 million open
  handles after a day of uptime) with no corresponding growth in scrollback or
  the glyph atlas. The Windows presentation rework listed under Changed above,
  together with the wgpu upgrade it carried, ended the leak; handle count is now
  flat under sustained rendering. This is the main reason to upgrade from 0.2.x.
- **Missing-letter rendering artifacts no longer persist.** Text runs and
  terminal grid rows that hit a transient glyph shaping/rasterization failure are
  no longer stored in the cross-frame caches, so a dropped glyph is retried on
  the next frame instead of replaying as a permanent hole app-wide. DirectWrite
  rasterizations that come back 0×0 now fall through to the swash rasterizer
  instead of silently dropping the glyph, and shaping failures are no longer
  negative-cached.
- **Glyph atlas eviction runs during sustained animation.** The periodic eviction
  check previously only existed on the slow redraw path, so 120hz fast-path
  streaks let the atlas fill monotonically toward exhaustion.
- **Text inputs now support the full selection hotkey suite.** In the
  rename-session dialog (and every other text field), `Ctrl+A` selects the whole
  value with a visible highlight, typing or `Backspace`/`Delete` replaces the
  selection, `Ctrl+C`/`Ctrl+X`/`Ctrl+V` copy/cut/paste it,
  `Shift+Arrow`/`Shift+Home`/`Shift+End` extend it, `Ctrl+Arrow` jumps by word
  (`+Shift` selects by word), `Ctrl+Backspace`/`Ctrl+Delete` delete by word, and
  double-click selects the word under the cursor. Previously the framework's
  inputs had no selection model at all, so `Ctrl+A` and friends silently did
  nothing.
- **Pointer hover changes at non-default Windows display scaling no longer
  restyle the entire document.** Narrow pseudo-class restyles inherit logical,
  unscaled parent styles and avoid compounding DPI scale while preserving scoped
  style work.
- **Sidebar terminal names no longer collapse to `…` next to a long git-branch
  chip.** The row now lets the branch chip shrink (down to a small floor) before
  the terminal name loses width, so longer names — including the new program-set
  titles — stay readable at the default sidebar width.
- **Timer-paced surfaces keep an absolute presentation phase** instead of
  accumulating wake-up drift. Optional render lead can absorb variable work while
  reporting its hold separately.
- **Input-latency capture now survives pacer rejections and empty animation
  frames.** Pending human input remains attached to the next frame that actually
  presents.
- **Active rendering no longer performs synchronous monitor queries or native
  title rewrites once per second.** Refresh-rate changes are reconciled on
  startup, move, scale, and focus events, while live FPS remains available
  through frame metrics and the in-app overlay.
- **Desktop automation ignores winit's transparent thread-event helper window
  when locating the real app HWND.** Real-input runs persist bounded focus
  diagnostics and tolerate transient Win32 activation changes instead of
  misdirecting keystrokes.

## [0.2.6] - 2026-07-16

Terminal interaction polish: links now show clear hover feedback, and navigation
key input reaches terminal applications again.

### Added

- **Terminal HTTP(S) links now show hover feedback.** Moving the pointer over a
  detected link underlines its complete span and changes the cursor to a hand,
  while opening it remains an explicit `Ctrl`+click action.

### Fixed

- **Terminal navigation keys now reach the active shell or application.** Plain
  `Shift`+`Tab` sends the standard terminal BackTab sequence, `Ctrl`+Arrow is
  available for word navigation, and pane-focus shortcuts use
  `Ctrl`+`Alt`+Arrow instead.

## [0.2.5] - 2026-07-11

Observability release: terminal panes can export a compact debug pointer plus a
full JSON terminal snapshot, the Sessions settings page now shows RAM usage for
the UI, daemon, and terminal sessions, and common colored status emoji render
with their canonical colors instead of foreground-colored tofu boxes.

### Added

- **Export terminal info from a tab name context menu.** Right-click the visible
  tab name and choose "Export terminal info" to write a full JSON snapshot under
  the profile data directory and copy a small pointer JSON to the clipboard. The
  export includes pane/session ids, workspace/tab context, terminal grid shape,
  cursor, scrollback and mode state, renderer metrics, mapped/cached PTY
  sessions, recent PTY events, and UI/daemon memory metadata. The pointer keeps
  clipboard payloads small while giving agents and debugging tools a stable file
  path to inspect.
- **RAM usage is now visible in Settings -> Sessions.** The sessions refresh path
  samples the UI process, daemon process, and each live terminal session working
  set, then shows total RAM, UI/ptyd/terminal buckets, per-workspace rollups, and
  per-session memory pills. Sampling is best-effort and keeps session listing
  reliable if a process exits or denies inspection.

### Fixed

- **Common colored terminal status emoji no longer render as solid
  foreground-colored tofu.** Colored squares/circles, ❌, ✅, and ⚠️ are now
  drawn as crisp canonical-colored vectors in the terminal renderer, matching
  the practical Windows Terminal look for AI CLI status markers without waiting
  on a full color-glyph atlas pipeline.
- **Inactive workspaces no longer show selected subtab styling.** Sidebar
  subtabs only receive active styling for the active workspace, so background
  workspaces no longer look selected.

## [0.2.4] - 2026-07-08

Quality-of-life release for terminal panes: Ctrl+click opens `http(s)` links in
your default browser, the mouse wheel now scrolls full-screen TUIs (Claude Code,
vim, fzf, lazygit) that capture the mouse, missing symbol glyphs (task-list
checkboxes, Braille spinners) render as crisp vectors instead of solid blocks,
and already-typed UI text no longer jitters while you type.

### Added

- **Ctrl+click a URL in a terminal pane to open it in your default browser.** `http://` and `https://` links printed by any program are now detected under the pointer; holding `Ctrl` and clicking one hands it to the system's default browser instead of starting a text selection (a plain click still selects, and `Ctrl`+drag over non-link text still selects). Detection keeps query strings and fragments intact and strips trailing sentence punctuation (so clicking `http://example.com).` opens the bare address). Only `http`/`https` are ever opened, and the URL is passed to the OS via the shell-association API (`ShellExecuteW`), never through a command interpreter, so a link from an untrusted source (e.g. an SSH session) cannot inject shell syntax.

### Fixed

- **The mouse wheel now scrolls TUIs that capture the mouse (Claude Code, vim, fzf, lazygit).** Full-screen programs run in the alternate screen and enable mouse tracking (DECSET 1000/1002/1003 + 1006 SGR), expecting the terminal to *forward* wheel notches to them so they can scroll their own content — exactly what Windows Terminal and VS Code do. This terminal parsed those modes and threw them away, and its wheel handler only ever moved its own scrollback, so while such a program was running the wheel appeared to do nothing (the local scrollback it moved was the irrelevant pre-program shell history). It now tracks mouse-reporting mode and, when active, encodes each wheel notch as an SGR (or legacy X10) mouse-button report and writes it to the PTY, so the program scrolls as expected. `Shift`+wheel remains an escape hatch that always scrolls local scrollback, matching the xterm convention.
- **Task-list checkboxes and other symbol glyphs no longer render as solid colored blocks.** JetBrains Mono (the only bundled font) lacks the ballot-box, geometric-square, and Braille codepoints that tools like Claude Code print, and with no glyph fallback wired the renderer rasterized a `.notdef` box filled with the cell foreground — the orange/yellow "solid block" checkboxes. These ranges (U+2610–2612 ballot boxes, the U+25A0–25FE / U+2B1B–2B1C squares, and U+2800–28FF Braille) are now drawn as crisp, cell-fitted vectors — outline squares, filled squares, check/cross marks, and Braille dot matrices — the same way block and box-drawing characters already were. Spinners built from Braille animate cleanly instead of flashing identical tofu boxes.
- **Already-typed text no longer vibrates while you type in UI fields.** UI text glyphs were drawn at a fractional, per-frame-drifting run origin against a nearest-sampled glyph atlas, so a sub-pixel shift of the origin (a scrolling input, a centered field, DPR-scaled chrome) resampled every glyph onto different physical pixels each frame. UI text now snaps its origin to the device pixel grid on both axes, holding each glyph on one stable column while preserving shaped kerning.

## [0.2.3] - 2026-07-07

Bugfix release: panes no longer inherit the Claude Code profile / provider
override of whatever launched the daemon, and quotes typed on dead-key
keyboard layouts reach the shell.

### Fixed

- **`claude`/`cc` run inside a pane no longer picks up the launcher's Claude Code profile.** The PTY daemon is long-lived and inherits the environment of whatever launched it. When that launcher was a Claude Code session or a provider-override wrapper (e.g. a z.ai/GLM profile exporting `ANTHROPIC_BASE_URL` + `CLAUDE_CONFIG_DIR`), those variables propagated into every spawned pane, so an agent started in a pane opened the launcher's profile instead of the user's default config — and stayed poisoned until the daemon restarted. The daemon now strips the profile/provider/session variables (`CLAUDE_CONFIG_DIR`, the `ANTHROPIC_*` base-url/auth/model overrides, and the `CLAUDECODE`/`CLAUDE_CODE_*` session markers) from every pane spawn, so each pane is a clean interactive shell.
- **Quotes typed on dead-key layouts now reach the terminal.** On keyboard layouts where `'` and `"` are dead keys (US-International, ABNT2), the committed character was silently dropped: pressing the quote key twice produced nothing, and quote-then-space sent a plain space. Both paths now forward the composed text to the shell, so `'`, `"`, and other dead-key accents (`~`, `^`, `` ` ``) can be typed normally.

## [0.2.2] - 2026-07-06

This release lets the app run without a GPU via a software-renderer fallback,
pastes clipboard images straight into terminal panes (Windows Terminal parity),
adds Quick Prompt image drag-and-drop, makes the tab strip configurable, and
isolates dev/test instances from the installed app through instance profiles.

### Added

- **Paste images into terminal panes** (Windows Terminal parity). When you press **Ctrl+V** (or Ctrl+Shift+V / Shift+Insert / right-click) and the clipboard holds a bitmap instead of text — e.g. right after a ShareX **Ctrl+Print** capture, Win+Shift+S, or a browser "Copy image" — the image is saved as a PNG under `%TEMP%\godly-paste\` and its path is pasted into the focused pane, quoted when it contains spaces. Agent CLIs such as Claude Code pick the path up exactly like a drag-and-dropped image file. Text on the clipboard still takes priority; repeated pastes of the same screenshot reuse the same content-addressed file.
- **Software/CPU-renderer fallback** so the terminal manager runs on machines without a usable GPU (headless servers over RDP, VMs without GPU passthrough, old hardware) instead of panicking at startup. When no hardware adapter is available the renderer now escalates: it tries the preferred backend (Vulkan on Windows), then all backends (catching a real D3D12/OpenGL GPU), and finally falls back to a software adapter — WARP on Windows/D3D12, lavapipe on Vulkan — reusing the entire existing renderer so the output looks the same.
- A new `AdapterTier` (`Hardware` / `Software`) classifies the active adapter. On `Software` the renderer automatically disables 4× MSAA (the dominant fill cost) and the backdrop-filter blur, and builds a lightweight quad shader (`quad_software.wgsl`) that fits software adapters' smaller vertex→fragment varying budget (60 components vs the full shader's 96) by dropping gradients/shadows/masks. Terminal text and panel backgrounds/borders render identically; only gradient/shadow chrome goes flat.
- `TM_FORCE_SOFTWARE_RENDERER=1` (or `UNSHIT_RENDER_TIER=software`) exercises the fallback on a GPU machine for testing; `UNSHIT_RENDER_TIER=hardware` disables it. The GPU-accelerated path is unchanged: same adapter selection, full shader, 4× MSAA.
- The Quick Prompt overlay can now attach images two new ways, in addition to the existing paste pipeline:
  - **Drag-and-drop** — drop one or more image files (PNG/JPEG) onto the window to attach them. Non-image drops (folders, text files, unsupported formats) are skipped, and a hint is shown when a drop contained no usable image.
  - **Clipboard paste** — press **Ctrl+V** while the overlay is open to attach an image from the clipboard. A paste with no image on the clipboard is a silent no-op.
  - Both paths reuse the existing pasted-image handling: full-resolution PNG plus thumbnail, content-addressed so duplicates are de-duplicated, with identical chips, submit, and cleanup behavior.
- Configurable horizontal tab strip in Settings → Appearance → **tabs**:
  - **Tab sizing** — `fixed` pins every tab to a configurable width, or `fit content` shrink-wraps each tab to its own label.
  - **Tab width** — a stepper (120–400px, default 200px) for the fixed width; hidden in fit-content mode where there is nothing to tune.
  - **Tab rows** — keep the historical `single` scrolling row, or wrap the strip onto `double`/`triple` stacked rows. In multi-row mode the tab bar grows downward (the terminal grid below shrinks) and the `>`-style horizontal overflow is dropped; once tabs exceed the row cap the strip scrolls vertically instead.
- Instance profiles isolate parallel app instances from each other. Every
  OS-shared resource — the `unshit-ptyd` daemon pipe, the notification pipe,
  and the config dir (`workspaces.json`, `quick_prompt.json`,
  `keybindings.json`, Quick Prompt worktrees) — is now namespaced by a profile:
  - The **installed app** keeps the unsuffixed defaults (`com.godly.terminal`,
    `\\.\pipe\unshit-ptyd-<user>`), so nothing changes for daily use.
  - **Repo builds** (`cargo run`, debug or release, any `target*` dir)
    automatically run in the `dev` profile with their own daemon, sessions,
    and config — dogfooding a work-in-progress build can no longer attach to
    the installed app's sessions or overwrite its workspace layout.
  - `TM_PROFILE=<name>` selects an explicit profile (`TM_PROFILE=default`
    forces the installed-app namespace); `TM_CONFIG_DIR` additionally
    redirects the config dir, which tests use to stay fully ephemeral.
  The window title shows the active profile (e.g. `terminal manager [dev]`).

### Changed

- The rename-session dialog now prefills the field with the session's current name and focuses the input on open (cursor at the end of the name), so you can edit or retype it immediately without clicking. Backed by two new framework primitives on `ElementDef`: `with_value` seeds an input's buffer once on mount (preserved across re-renders so edits are never clobbered) and `with_autofocus` focuses an element the first time it mounts.
- Tabs now default to a fixed 200px width (previously a 150–240px content-clamped band). Width, sizing mode, and row mode are all adjustable from the appearance settings and reset with the rest of the appearance section.
- The software/CPU-renderer fallback now uses **grayscale antialiasing** for text instead of subpixel (ClearType) rendering. Subpixel text is a per-pixel cost — the subpixel shader samples three chroma channels and DirectWrite rasterizes RGBA coverage — that a CPU rasterizer (WARP/lavapipe) pays in full fragment shading. On the Software tier the renderer now builds an R8 (single-channel) glyph atlas and the grayscale `text.wgsl` shader unless `TM_FORCE_SUBPIXEL_TEXT=1` overrides it, so text-heavy terminal frames shade fewer fragments on non-GPU machines. The hardware path keeps the platform policy (ClearType on Windows) unchanged.
- The software/CPU-renderer fallback now renders **box-shadows** (outer and inset), restoring panel depth so the non-GPU path looks much closer to the GPU-accelerated one. The lite quad shader (`quad_software.wgsl`) was expanded with the full shader's shadow math — outer-spread expansion in the vertex stage, the tanh-Gaussian outer/inset shadow passes, and shadow compositing behind the rect — while staying within software adapters' 60-component varying budget (it now uses ~36 of 60; gradients and `mask-image` remain omitted). The GPU path and its full shader are unchanged.

### Fixed

- The `unshit-ptyd` PTY daemon is now built as a Windows GUI-subsystem binary in
  release, so launching the installed app no longer pops a stray console window
  alongside it. Previously the daemon was a console-subsystem executable and,
  depending on how Windows honored the `CREATE_NO_WINDOW | DETACHED_PROCESS`
  spawn flags, could surface its own terminal window next to the app. Debug
  builds keep their console so `cargo run -p unshit-ptyd` still shows logs, and
  the `--status` / `--version` / `--help` / `--shutdown` subcommands still print
  when run from a terminal (via `attach_parent_console`, mirroring the UI binary).
- Hardware ClearType (subpixel) text no longer renders a reversed colored fringe. The swash subpixel rasterizer emits coverage in **BGR** order, but the glyph atlas is sampled as RGBA where the red channel drives the left physical subpixel on a standard RGB display, so the data was being read reversed — measured per-pixel as a cyan/blue-left, red/orange-right halo on every stem (the opposite of correct RGB ClearType, and the hue contamination on colored text). The `SubpixelMask` atlas-fill path now swaps R↔B so red coverage lands in the red channel, matching the DirectWrite path (which already emits RGBA). Verified per-pixel after the fix: the left stem edge is now red-dominant (correct RGB orientation). The grayscale (R8) software path is unaffected.
- The software/CPU-renderer (grayscale text) path no longer paints a fake colored fringe on every glyph. The grayscale `text.wgsl` shader was synthesizing per-channel "subpixel" coverage by sampling the ±1 neighbour texels of a single-channel (R8) atlas into the red/blue channels — but a grayscale mask has no real subpixel data, so this only injected a cyan-left / orange-right halo on every stem and shifted the hue of colored text at its edges. The shader now samples the true coverage once and blends it straight (verified per-pixel: glyph edges are now neutral dimmer-foreground instead of color-fringed). `grid_fragment.wgsl` applies the same mild stem-contrast curve so terminal-cell text and UI text share identical grayscale weight. The hardware ClearType (`text_subpixel.wgsl`) path is unchanged.
- UI/chrome text (sidebar, tabs, breadcrumbs, status bar, buttons) now snaps its glyph baseline to a whole device-pixel row, so horizontal stems land on one crisp row instead of smearing across two at partial coverage on non-integer display scales (e.g. 1.5x). Positions are already in device pixels (font sizes are pre-scaled by the DPR); only Y is rounded (X is left untouched to preserve shaping/kerning), mirroring the trick the terminal grid path already uses (`gy.round()`). This path is UI-only — terminal cells render through their own emit path and are unaffected.
- The bottom status bar no longer renders the unreadable token `k/sutf-8`. The left and right status groups were laid flush against each other (`.statusbar` is `justify-content: flex-start; gap: 0`), so the left group's last item (`↓ 0.0 k/s`) collided with the right group's first (`utf-8`). A flex spacer (`.sb-spacer`, matching the settings status bar) is now inserted between the two groups, pushing the right group to the far edge as intended.
- Test harnesses and helper scripts can no longer disturb a running session:
  - `cargo xtask desktop-regression` launches every app session in a unique
    throwaway profile (own daemon pipe, temp config dir) and its pre-build /
    post-test process cleanup now matches executables by *path* (repo
    `target\debug` builds only) instead of killing every `terminal-manager.exe`
    / `unshit-ptyd.exe` by name — the installed app and its daemon are never
    collateral damage.
  - `scripts/kill-all.ps1` is repo-scoped by default (only kills processes
    running from this repository's build dirs) and requires `-All` to touch
    anything else.
  - Screenshot helpers (`palette-shot.ps1`, `software-renderer-shot.ps1`) run
    the app in an ephemeral profile via `scripts/lib/tm-isolation.ps1` and shut
    their daemon down afterwards.

## [0.2.1] - 2026-07-05

Pre-release test build of the non-GPU/software-renderer channel, published as
`terminal-manager-0.2.1-non-gpu-setup.exe` alongside the official 0.2.0 build.
Its changes are folded into the 0.2.2 entry above.

## [0.2.0] - 2026-06-17

This release makes terminal scrolling smooth and its frame timing honest, adds
mouse selection / copy / paste to the terminal, and restyles the workspace menu
and Keybinds settings to the design system.

### Added

- Mouse text selection in the terminal: click-drag to select, double-click for a (path-aware) word, triple-click for a line, and Shift+click to extend. Selections are anchored to absolute buffer lines, so they stay pinned to their text as the view scrolls and as output streams; copying always returns the highlighted text.
- Copy following Windows Terminal conventions: `Ctrl+C` copies when there is a selection (and still sends `SIGINT` when there is none), `Ctrl+Shift+C` always copies. Right-click and `Shift+Insert` paste, and bracketed paste (DECSET 2004) wraps pasted text in `ESC[200~`/`ESC[201~` when the running program enabled it.
- Animated terminal scrolling (scroll-smoothness spec Phase 3): wheel notches now ease the scrollback view over ~180ms with the same browser-validated curve as the settings page, retargeting in-flight on new notches instead of teleporting whole rows. Content renders with sub-row, device-pixel-snapped precision via a one-row overscan snapshot and a paint-time translation, so motion is continuous rather than row-quantized.
- Fractional wheel-scroll accumulation (Phase 2): wheel and touchpad deltas accumulate with sub-line precision instead of rounding every event away from zero, so scroll distance is exactly proportional to input — no more 7-rows-per-notch over-travel or 5× touchpad amplification.
- Scrolled-back viewport anchoring: streaming PTY output no longer snaps the view to the live bottom while you read scrollback; the view (and any in-flight scroll animation) shifts with scrollback growth, including at-capacity eviction. Entering or leaving the alternate screen (full-screen TUIs) still snaps to live.
- Vblank-anchored frame pacing (Phase 4): on a vsync-paced surface the renderer's blocking swapchain acquire now anchors the paint loop to the display's refresh clock, so animation frames land one-per-refresh instead of being driven by a wall-clock timer that beats against scanout. The swapchain prefers `Fifo` (guaranteed on Vulkan) over Mailbox/Immediate; surfaces without a blocking present mode fall back to true-period timer pacing. A `UNSHIT_FRAME_LATENCY` env var (`1` or `2`, default `1`) A/Bs the swapchain's maximum frame latency without a rebuild.
- Honest presentation-cadence metrics (Phase 1): the FPS overlay now reports fps as paints within the trailing second instead of `1/work_time`, with `p50/p95/p99/max` present-interval rows and a `dropped` counter; a once-per-second `[FRAME-INTERVAL]` log line emits cadence quantiles. Idle cadence breaks (e.g. the cursor-blink repaint) are excluded so an idle session does not fabricate jank.

### Changed

- Redesigned the sidebar workspace right-click menu to a "submenu flyout" layout: each action row leads with an icon and (for navigational actions) a keyboard-hint badge, the shell list moved into a hover flyout that spawns beside "New terminal" with favourite shells starred, and the destructive actions (Kill all terminals, Remove workspace) are fenced into a grouped danger zone. The menu uses Windows-legible keyboard hints.
- Restyled the Settings → Keybinds page to match the design mockup: a grouped command list, a filter input, and keycap-style key pills. The Settings page now toggles closed when `Ctrl+,` is pressed again while it is open.
- The frame pacer no longer emulates vsync or "timer-compensates" 120Hz down to 8ms; it reports the display's true refresh period (e.g. 8.333ms at 120Hz, 16.666ms at 60Hz) and survives only as the metrics floor and the Timer-fallback redraw coalescer. Sub-10Hz refresh reports are treated as driver garbage and fall back to the 8ms default.
- A single persistent, deadline-extended animation waker replaces the per-wheel-notch waker threads; terminal scroll and container smooth scroll tick from the same shared motion module. On the default vsync-paced path the waker thread is never spawned — the blocking acquire is the tick.
- Mouse-wheel notch normalization now divides unconditionally by the OS wheel setting (`SPI_GETWHEELSCROLLLINES` / `SPI_GETWHEELSCROLLCHARS`, queried per event), removing the 3× amplification of sub-notch deltas from high-resolution wheels; one detent always scrolls exactly the configured distance.
- Wheel scrolling over the terminal now updates grid content as a paint-only patch, so it no longer forces a full UI tree rebuild or interrupts a concurrent smooth-scroll animation. A visible FPS overlay requests rebuilds at most ~4Hz instead of every painted frame.
- The UI framework's `DragEvent` and `MouseEvent` now carry element-local pointer coordinates (`local_x` / `local_y`) and `MouseDown` is dispatched to element handlers, so grid/canvas widgets can map a pointer to a cell without re-deriving their absolute rect; a `Key::Insert` variant was added. The CSS/layout engine now measures elements that mix raw text and child elements correctly via anonymous text boxes.
- Bench report JSON: `fps_mean` was renamed to `paints_per_sec_mean`, with new `interval_p50/p95/p99/max_ms`, `interval_stddev_ms`, `judder_ratio`, and a present-interval `interval_histogram`. The experimental `grid-fragment-shader` path no longer applies to terminal grids (no overscan support); terminal grids render through the standard cell emitter unconditionally.

### Fixed

- Reconciler: when a matched child's tag changed, the child chain was stitched with the old (deallocated) `NodeId`, silently truncating the sibling chain — this blanked the entire content column after closing settings (Esc or repeated `Ctrl+,`) and rendered keybind rows / the filter input blank on the restyled Keybinds page. `reconcile_inner` now returns the live `NodeId`, covered by keyed and unkeyed tag-change regression tests.
- Swapchain acquire failures now follow an explicit, unit-tested recovery policy: `Lost` always reconfigures, `Outdated` reconfigures at most once per episode and never while the window is minimized (preventing a reconfigure storm and an unvalidated stale-extent submit on minimized Vulkan windows), and timeouts / other errors drop the frame without touching the surface.
- Settings: keybind key pills no longer overflow their chips.

## [0.1.0] - 2026-06-07

Initial release of Terminal Manager — a GPU-accelerated, agentic terminal manager for Windows.

### Added

- Initial release of Terminal Manager — a GPU-accelerated terminal manager for Windows, with real PTY-backed terminals rendered through a `wgpu` pipeline.
- Terminal multiplexing: workspaces with tabs, splittable panes (split right / down), and resizable split dividers.
- Session persistence: the full layout (every workspace's tabs, pane splits, split ratios, and pane ids) is saved and restored on restart, reattaching each surviving `unshit-ptyd` session — including agent tabs — to the pane it was in.
- Command palette (`Ctrl+Shift+P`, with `Ctrl+K` as an alias): a VS Code-style palette with grouped results, keyboard/mouse selection, preview details, footer hints, and modes for commands (`>`), agents (`@`), navigation (`:`), and scrollback (`/`).
- Command palette actions to rename the current terminal, split panes right or down, open a new terminal, close the current pane, toggle the sidebar, and open settings, with honest empty states when no source data is available.
- Quick Prompt overlay (`Ctrl+Shift+Q`): a centered prompt where you pick Claude Code or Codex CLI and dispatch a task into a fresh isolated git worktree where the agent runs unattended.
- Window controls: native titlebar minimize/maximize/close buttons, custom window resize cursors, and maximized-state reflection in the titlebar.
- Cursor rendering with blink behavior and correct first-frame alignment.
- A from-scratch CSS/layout engine driving the UI, supporting `transform` (`scale` / `scaleX` / `scaleY`, `rotate`, `translate` / `translateX` / `translateY`, composed about the box center and animated via transitions and `@keyframes`), `text-shadow` colored glows rendered without offscreen render targets, and `text-overflow: ellipsis` that truncates correctly across LTR, RTL, bidi, and combining-mark/ZWJ-emoji text.
- CSS engine `calc()` evaluation for length values (`length ± length`, `length × / ÷ number`, nested parens and precedence), resolving viewport-relative constraints such as `max-width: calc(100vw - 48px)` at layout time.
- CSS engine support for viewport and percent units in `padding`, percentage `border-radius`, per-side `border-*-color` longhands, per-axis `overflow-x` / `overflow-y`, the `outline` shorthand, `font-style: italic`/`oblique` on DOM text, and `justify-content` extensions (`stretch`, `left`/`right` aliases).
- A Windows desktop-regression test harness (`cargo xtask desktop-regression`): a Rust runner with headed app launch, Win32 window control, screenshots, runner events, versioned `desktop-regression.results/v1` artifacts, and failure bundles.
- A token-authenticated `terminal-manager` diagnostics protocol over Windows named pipes, exposing snapshots (terminal cursor, scrollback length, active session id, PTY mappings, renderer frame/present state, dirty regions, and an opt-in `buffer_window`), step markers, invariant evaluation, deterministic-mode prep, and event draining.
- Record-and-replay support for desktop regression suites, named run profiles with preflight checks and suite/tag filtering, owned-process lifecycle tracking, bounded wait/retry primitives, and interactive failure inspection.
- A `stylesheet_coverage` guardrail that records every declaration the engine cannot type and fails the build when the app stylesheet grows an undocumented gap.

### Changed

- `var()` is now cascade-aware: custom properties resolve per element against an ordered scope chain (self → active `.app.theme-*` root → `:root`) with multi-level token aliasing, so per-theme token overrides finally apply (custom-property drops fell from 579 to 0).
- `transition` lists no longer drop entirely when they name a not-yet-animatable property; `transform` is now an animatable property, and unrecognized entries are skipped individually.
- The renderer carries each element's transform as a delta-from-identity 2x3 affine propagated through the subtree, retiring the previous `translateX`-only render offset; untransformed elements stay on the matrix-free fast path.
- A range of authored-but-inert CSS properties (`appearance`, `-webkit-font-smoothing`, `border-collapse`, `background-repeat`, `font-feature-settings`, `scrollbar-width`, `text-shadow: none`, `background: none`, etc.) are now accepted and intentionally ignored per CSS forward-compat semantics instead of being dropped.
- Present modes now prefer low-latency options, and settings appearance, scrolling, theme preview, and cursor/output sync were polished.
- Desktop regression suites and templates were migrated from the legacy PowerShell runner to the Rust runner (PowerShell entry points remain compatibility wrappers), use shared session/capture/assertion/wait helpers, and emit collision-resistant run ids.

### Fixed

- Restored full layout and session restoration on restart: every close path (the "keep running" / "kill & quit" dialog and the remembered silent-close preference) now persists the layout, so sessions are no longer orphaned on the daemon after relaunch.
- Fixed the command palette not matching its design: accent/density `--cp-*` tokens were only collected from `:root`/`*` (now declared there), and the active-row rail and `12vh` top offset relied on then-unsupported `border-left-color` and viewport-unit padding.
- Fixed scroll containers that stopped scrolling: the `overflow` shorthand and recognized-but-inert accept arms over-consumed the declaration stream and dropped the following declaration (e.g. `height` after `overflow: scroll`); both now stop exactly at the declaration terminator.
- Fixed a `:root` comment containing `:` silently dropping the custom property declared after it (notably `--cp-accent`), by stripping comments before the custom-property pre-scan.
- Fixed the `text-shadow` glow blowing out to a bright smear on the Windows subpixel text path by folding `color.a` into the premultiplied shader output, so glow intensity tracks the shadow's alpha and translucent text composites correctly.
- Stabilized split divider and edge resize handling, including the edge-resize restore flow that previously failed setup on a no-op restore drag.
- Hardened the desktop regression harness: traces are now consumed (not just validated) for supported suites, the app only advertises diagnostic event families it actually emits (`test_step`, `invariant`, `log`), `--observe basic` runs write `pre-snap`/`post-snap` snapshots, and the `post-resize-glitches` suite fails on a blank mid-pane, lost foreground, stuck modifier, or overlapping non-owned window.
- Fixed terminal blanking after a snap resize.

[Unreleased]: https://github.com/alangmartini/unshit-agentic-terminal-manager/compare/v0.3.3...HEAD
[0.3.3]: https://github.com/alangmartini/unshit-agentic-terminal-manager/compare/v0.3.2...v0.3.3
[0.3.2]: https://github.com/alangmartini/unshit-agentic-terminal-manager/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/alangmartini/unshit-agentic-terminal-manager/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/alangmartini/unshit-agentic-terminal-manager/compare/v0.2.6...v0.3.0
[0.2.6]: https://github.com/alangmartini/unshit-agentic-terminal-manager/compare/v0.2.5...v0.2.6
[0.2.5]: https://github.com/alangmartini/unshit-agentic-terminal-manager/compare/v0.2.4...v0.2.5
[0.2.4]: https://github.com/alangmartini/unshit-agentic-terminal-manager/compare/v0.2.3...v0.2.4
[0.2.3]: https://github.com/alangmartini/unshit-agentic-terminal-manager/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/alangmartini/unshit-agentic-terminal-manager/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/alangmartini/unshit-agentic-terminal-manager/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/alangmartini/unshit-agentic-terminal-manager/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/alangmartini/unshit-agentic-terminal-manager/releases/tag/v0.1.0
