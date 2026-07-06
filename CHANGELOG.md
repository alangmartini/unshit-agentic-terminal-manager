# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/alangmartini/unshit-agentic-terminal-manager/compare/v0.2.2...HEAD
[0.2.2]: https://github.com/alangmartini/unshit-agentic-terminal-manager/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/alangmartini/unshit-agentic-terminal-manager/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/alangmartini/unshit-agentic-terminal-manager/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/alangmartini/unshit-agentic-terminal-manager/releases/tag/v0.1.0
