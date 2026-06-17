# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/alangmartini/unshit-agentic-terminal-manager/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/alangmartini/unshit-agentic-terminal-manager/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/alangmartini/unshit-agentic-terminal-manager/releases/tag/v0.1.0
