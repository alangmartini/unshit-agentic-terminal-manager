# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/alangmartini/unshit-agentic-terminal-manager/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/alangmartini/unshit-agentic-terminal-manager/releases/tag/v0.1.0
