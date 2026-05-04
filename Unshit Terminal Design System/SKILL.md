---
name: terminal-mgr-design-system
description: Design system for terminal.mgr — a retro-amber CRT-on-walnut Rust terminal manager. Use this skill when designing any UI surface (app screen, marketing site, slide, doc cover, prototype) that should match the terminal.mgr aesthetic.
---

# terminal.mgr design system

A warm, dim, monospaced design system for a Rust-based agentic terminal manager. Walnut darks, amber ember accents, scanline overlay, no sans-serif. Source repo: `alangmartini/unshit-agentic-terminal-manager` @ `feat/rust-terminal-manager`.

## When to use

- New screens or features for the terminal.mgr desktop app
- Marketing surfaces (landing, docs cover, changelog)
- Slides, decks, internal product docs that should look on-brand
- Any prototype that should slot into the existing visual language

## How to use

1. **Read `README.md`** at the project root first — it covers voice, casing, color philosophy, type, animation, iconography, layout rules, and what to flag for substitution. The voice section in particular is non-negotiable: lowercase, mono, no emoji, technical, slightly profane in source code, never marketing-fluffy.
2. **Pull tokens from `colors_and_type.css`** — never invent new colors, fonts, spacing, radii, or shadows. The amber ramp + four semantic accents (sage / rust / ember / azure / violet) is the entire palette.
3. **Reuse components from `ui_kits/terminal_manager/`** — Titlebar, Sidebar, TabStrip, PaneGrid, Statusbar, SettingsModal, Palette. The `kit.css` covers the chrome; `index.html` is a working app shell you can fork.
4. **Reuse icons from `assets/icons/icons.svg`** — 23-symbol inline-SVG sprite at 16×16, 1.4–1.6 stroke, `currentColor`. Don't redraw existing icons; if a new one is needed, match the conventions.
5. **Always ship the ambient layer** on full-app surfaces: two soft radial gradients (warm amber top-right, deeper rust bottom-left) + repeating-linear-gradient scanlines at 22% opacity, `mix-blend-mode: multiply`. Without this the system loses ~30% of its identity.

## Hard rules

- **Mono everywhere.** JetBrains Mono primary, Berkeley Mono fallback. No sans-serif ladder. Body, headings, labels, terminal output, brand mark — same family.
- **No emoji.** Anywhere. Use the unicode glyphs in the typographic vocabulary (`❯ ▾ ▸ ◆ × ├ └`) for prompts, chevrons, marks, tree lines.
- **No drop shadows on cards.** The active-pane treatment is darkening (inset + outer dark), never lifting.
- **Rails over capsules** for "currently selected" state. Capsules are reserved for tags, status badges, kbd hints.
- **Glows are tinted to match their underlying color.** No white shadows in this system.
- **Lowercase by default.** Title Case only inside settings labels and modal section descriptions.
- **Fast motion only.** Three durations: 120ms / 200ms / 320ms. Two easings. Cursor blink is `steps(2)` — hard on/off, no fade.

## Preview cards

The Design System tab renders 16 cards covering surfaces, amber ramp, semantic accents, type specimens (display / UI / terminal palette), spacing scale, radii + shadows, buttons, inputs, badges + chips, session rows, command palette, pane chrome, and the logo lockup. Inspect those before designing anything new — they show the exact visual rhythm to match.

## Substitutions to flag with the user

- **Berkeley Mono** is paid. The local font cascade picks it up if installed; otherwise JetBrains Mono ships from Google Fonts. Berkeley tightens the rhythm noticeably.
- **No hero imagery exists.** If a marketing surface needs a hero, ask before inventing — the source has none. Compose a real terminal screenshot inside the scanline + amber-glow background as a placeholder.

## Layout fixed dimensions

- Titlebar: 34px · Tabbar: 38px · Statusbar: 24px · Sidebar: 252px · Resizer: 4px
- Modal: 860 × 76vh, min 520, max 760
- Inner gaps: 4–16px universally; 32px (sp-10) reserved for major section breaks
