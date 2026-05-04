# Unshit Terminal Manager — Design System

A retro-amber, CRT-on-walnut terminal manager built in Rust on the in-house `unshit` framework. This design system captures the visual + content fundamentals so any new screen, slide, or marketing surface lands in the same warm, grain-flecked aesthetic.

> *"Trying to make a terminal manager that isn't another shitty electron app."*
> — repo description, sets the entire tone.

---

## Sources

- **Primary repo (UI source of truth):** `alangmartini/unshit-agentic-terminal-manager` @ `feat/rust-terminal-manager`
  - `index.html` — full app shell markup (titlebar, sidebar, tabs, panes, settings modal)
  - `styles.css` — all CSS tokens + component styles, mirrored into `colors_and_type.css` here
  - `app.js` — interaction model (split panes, resizers, tabs, settings)
  - `CLAUDE.md`, `SPEC.md`, `specs/*.md` — content + voice reference
- **Framework:** `crates/unshit-framework/` (subtree from `alangmartini/unshit-rust-framework`) — CSS-syntax UI engine for Rust, GPU-accelerated via wgpu + Yoga
- **Daemon:** `crates/unshit-ptyd/` — owns PTYs and sessions; survives UI restarts

The product is a **single-product** system right now: one desktop app. The marketing site, slides, and any future docs all share the same chrome.

---

## What this is

A terminal manager — multi-pane, multi-workspace, multi-agent. The user model:

- **Workspaces** group terminals by repo / branch / project (`main`, `api`, `infra`, `scratch`)
- **Tabs** are individual terminal sessions inside a workspace
- **Panes** split tabs horizontally / vertically, up to 4×4
- **Agents** (claude, amp, codex) attach to terminals and run autonomously; a sidebar **activity feed** shows their state (`running`, `stopped`, `waiting`)
- **Sessions** survive UI restarts via the `unshit-ptyd` daemon

---

## Index

```
README.md                  ← you are here
SKILL.md                   ← Claude Code skill manifest (copy as agent skill)
colors_and_type.css        ← tokens: surfaces, borders, text, accents, type, motion
assets/                    ← logos, generic preview screenshots, brand imagery
preview/                   ← design system cards (read by the Design System tab)
ui_kits/
  terminal_manager/        ← high-fidelity prototype recreation of the app
    index.html             ← interactive demo: click panes, split, open settings
    *.jsx                  ← componentized chunks (Titlebar, Sidebar, Pane, ...)
    README.md              ← what's in this kit, what's faked
```

---

## Content fundamentals

### Voice

Direct, technical, slightly profane. The repo name (`unshit-agentic-terminal-manager`) and tagline (`isn't another shitty electron app`) set the register: this is an opinionated developer tool by a developer who's tired of bloat. We don't apologize for being technical. We don't say "powerful" or "delightful." We say what the thing does.

**Examples in source:**
- Repo desc: `"Trying to make a terminal manager that isn't another shitty electron app."`
- `CLAUDE.md`: `"We own the unshit framework precisely so we can fix its bugs and limitations upstream."`
- `CLAUDE.md`: `"App-level fixes are reserved for app-specific concerns."`
- `SPEC.md`: `"Today the daemon always falls back to pty::default_shell() because the UI never fills the protocol's shell field."` — diagnostic, not euphemistic

### Casing

**lowercase by default**, everywhere user-visible:
- nav: `workspaces`, `terminals`, `agents`, `worktrees`, `sessions`, `environment`
- modal nav: `general`, `appearance`, `shell`, `keybinds`, `agents`
- modal sections + footers, hints, status badges (`running`, `stopped`, `waiting`, `idle`, `disabled`)
- buttons: `cancel`, `save changes`, `reset to defaults`

Title Case appears only in **setting labels and descriptions** inside the modal (`Default shell`, `Restore on startup`, `Confirm before closing`). One-line sentences, no trailing period in labels, **trailing period optional** in descriptions (source uses none).

The brand name itself is `terminal.mgr` — lowercase, dot-separated, no version inflation.

### Person

- Documentation: imperative + technical (`Run cargo test`, `Write the failing test first`)
- UI copy: imperative + concise (`New terminal`, `Close tab`, `Reset to defaults`)
- No "we" / "you" hand-holding. No marketing fluff.

### Emoji

**Never.** Not in UI. Not in docs. Not in commit messages where it can be helped. The visual language already carries warmth via amber + glow; emoji breaks the CRT illusion.

### Unicode glyphs (used liberally)

- `❯` — prompt arrow (amber-300 with glow)
- `▾` `▸` — workspace chevrons
- `├` `└` — tree glyphs in the sidebar (faux ASCII tree)
- `◆` — modal mark + status accent (the "active workspace" diamond)
- `×` — close button on tabs
- `⌘` `⏎` `↑↓` — keybind hints (real, not mocked)
- `→` `↓` — log direction indicators

### Numbers + technical strings

- Tabular numerics everywhere (`font-variant-numeric: tabular-nums`)
- Paths render in azure: `~/main/dashboard`, `http://localhost:4040`
- Branches in sage: `(main)`, `(fix/pdf-export)`
- Times in dim: `[14:32:07]`
- Commands in primary cream: `go run main.go --port 4040 --watch`

---

## Visual foundations

### Color philosophy

Warm dark, never neutral. The base surfaces tilt toward walnut (`#1c1812` — warm brown-black) instead of grey. The amber accent reads as ember in wood, not neon on chrome. Six-step amber ramp + four secondary semantic accents (sage / rust / ember / azure / violet). All glows are **tinted** to match the underlying color — there are no white shadows in this system.

### Type

Mono. **Everywhere.** `JetBrains Mono` primary, `Berkeley Mono` (paid, common dev font) as preferred local fallback, then SF Mono / Menlo / Consolas. There is no sans-serif ladder. Body copy, UI labels, modal titles, button text, brand name — all in the same monospace family. Size range is tight: 10/11/12/13/14/16px. The default is 12px (`--t-md`); 14px is a heading; 16px is reserved for modal titles.

Active text gets `text-shadow: 0 0 8px rgba(...)` to glow softly. Tabular numerics on by default.

### Backgrounds

1. **Surface base** — solid walnut (`--bg-base`), no patterns.
2. **Ambient overlay** — two soft radial gradients (warm amber top-right, deeper rust bottom-left) at 3–5% opacity.
3. **Scanline overlay** — 1px-on-2px repeating horizontal lines at 22% opacity, `mix-blend-mode: multiply`. This is the CRT signature. Without it, the system loses 30% of its identity. **Always include both layers** on any full-app surface.
4. **Pane subtle vignette** — `radial-gradient(ellipse at top, rgba(212,163,72,0.015), transparent 60%)` adds 1.5% amber fade at pane top.

There are **no full-bleed images, no hand-drawn illustrations, no stock photos**, no big hero gradients. The grain + scanlines are the texture.

### Animation

Fast, decisive, never bouncy. Three durations only: `120ms` / `200ms` / `320ms`. Two easings: `cubic-bezier(0.22, 0.61, 0.36, 1)` (default) and `cubic-bezier(0, 0, 0, 1)` (out-only, for modal entry). Common transitions:

- Hover: 120ms color + background swap
- Focus / select: 200ms border + shadow
- Modal entry: 200ms fade + 12px lift, ease-out
- Cursor blink: 1.1s `steps(2)` infinite (hard on/off, no fade — feels like a real terminal cursor)
- Pulsing dot (sidebar agent indicator): 2s ease-in-out infinite, opacity-only

`prefers-reduced-motion: reduce` collapses everything to 0.01ms.

### Hover + press states

- **Hover:** raise background by one step (`--bg-subtle` → `--bg-hover`), bump text up one step (`--fg-secondary` → `--fg-primary`). No translate, no scale.
- **Active workspace / pane / tab:** swap to `--bg-elevated`, recolor text amber-100, drop a 2px amber-300 left rail with `box-shadow: 0 0 10px rgba(amber, 0.6)` glow.
- **Press (icon-btn):** `transform: scale(0.94)` for 120ms.
- **Primary button press:** `translateY(1px)`.
- **Focus-visible:** 1px `--border-focus` outline, 1px offset.

### Borders

Five tokens: `hair`, `soft`, `default`, `strong`, `focus`. Hair lines (`#241e15`) almost vanish — used for chrome dividers. Soft (`#342b1e`) for control borders. Default (`#4a3e2a`) for the active pane. Focus (`#d4a348`) is the amber-300 — only on focused inputs / focus rings.

### Inner / outer shadow systems

- Outer shadow: `--shadow-sm` (1px), `--shadow-md` (4px lift), `--shadow-lg` (14px modal)
- Inner shadow: used on toggles in the "on" state (`inset 0 0 8px rgba(212,163,72,0.22)`) and on focused inputs (`box-shadow: 0 0 0 2px rgba(212,163,72,0.12)`)
- Glows (separate concept): `--glow-amber` for active pane corners, `--glow-sage` / `--glow-rust` for status dots

### Layout rules

- Fixed: titlebar `34px`, statusbar `24px`, sidebar `252px`, tabbar `38px`, resizer `4px`
- Grid: app shell uses `display: grid` with named row/column tracks (`grid-template-rows: var(--titlebar-h) 1fr`)
- Modal: `860px × 76vh`, `min-height 520px`, `max-height 760px`
- Inner gaps: 4–16px almost universally; the 32px `--sp-10` is reserved for major section breaks
- Density: deliberately tight. Tab bar 38px, pane header 26px, status bar 24px

### Transparency + blur

Used sparingly:
- Modal scrim: `rgba(10, 8, 6, 0.78)` + `backdrop-filter: blur(8px)`
- Modal panel: 98% alpha gradient over walnut
- Modal nav rail: 44% alpha walnut over the modal — gives a subtle inner cardstock layer
- Pane header active rails, focus shadows: never blurred

No glassmorphism. No frosted icons. No transparent panes.

### Imagery vibe

Warm, dim, grainy. Anything photographic should feel like it's behind smoked glass — desaturated, low contrast, amber-biased. Most surfaces have **no imagery at all**; the grain + glow IS the imagery.

### Corner radii

Almost square. `--r-sm` = 2px (default for inputs, badges, key caps). `--r-md` = 3px (icon buttons, modal nav items, agent icon tiles). `--r-lg` = 6px (modal). `--r-xl` = 8px (rarely used; reserved for special cards).

### Cards

Cards are rare. When they appear they are minimal: 1px `--border-soft` border, `--bg-subtle` or `--bg-elevated` fill, `--r-md` corners. **No drop shadow on cards** — the active pane uses `box-shadow: inset 0 0 0 1px rgba(212,163,72,0.12), 0 0 20px rgba(0,0,0,0.25)` which is darkening, not lifting. The grid surface itself is a card; nesting cards inside cards is forbidden.

### Capsules vs rails

- **Rails (left-edge stripe)**: active workspace, active subtab, active pane header, active modal-nav item. Always 1–2px wide, amber-300 or amber-400, with a tinted glow.
- **Capsules (rounded-pill chips)**: branch tags (sage), agent badges (sage / amber / muted), keyboard hints (`<kbd>`).

The system **prefers rails over capsules** for "currently selected" — capsules are reserved for tags, status, kbd hints.

---

## Iconography

**Custom inline SVG.** No icon font, no Lucide / Heroicons CDN, no PNG. Every icon in the source app is hand-rolled `<svg viewBox="0 0 16 16">` with `stroke="currentColor"`, `stroke-width: 1.4–1.6px`, `stroke-linecap: round`, `stroke-linejoin: round`, no fill.

**Standard size:** 11px–14px, inline. They inherit color from the parent (`color: var(--fg-tertiary)` by default, amber-200/300 on active states).

**Stroke convention:** all icons share weight (1.4–1.6) and are designed to read at 11px. They are pictograms, not illustrations: terminal `>`, gear, plus, chevron, fullscreen corners, etc.

**No emoji. No unicode for icons** (unicode IS used for the prompt `❯`, tree glyphs `├ └`, mark `◆`, chevrons `▾`, but those are typographic glyphs sitting in mono runs — they're not icons).

The `assets/icons/` folder ships the canonical 16-frame SVG set extracted from the source. When using the design system in a new surface: copy from there, don't redraw. If a needed icon isn't in the set, draw a new one matching the conventions (16x16 viewBox, 1.4–1.6 stroke, round caps, `currentColor`).

---

## Substitutions to flag

- **Berkeley Mono** is paid; we ship the JetBrains Mono import as the public default. If a designer has Berkeley Mono installed, the local font cascade picks it up first and the design tightens noticeably — Berkeley has narrower rhythm than JetBrains.
- **Preview screenshots** in `assets/preview-*.png` are PNGs from the repo, used as reference for the prototype kit.

If you are building a marketing surface and need a hero image, **ask** — there isn't one in the source. Draw a placeholder with the scanline + amber-glow background and a real terminal screenshot composited inside.
