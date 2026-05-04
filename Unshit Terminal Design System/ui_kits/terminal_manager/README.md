# terminal.mgr — UI Kit

Hi-fi prototype recreation of the unshit-agentic-terminal-manager desktop app, built from the source repo's `index.html` + `styles.css`. Use this as the reference for any new screen, marketing surface, or downstream design surface.

## What's in here

- `index.html` — the full app shell: titlebar, sidebar, tab strip, 2×2 pane grid, status bar, plus an interactive command palette (`⌘K`) and settings modal (`⌘,`).
- `kit.css` — component-level styles (the app shell only). All design tokens are pulled from `/colors_and_type.css` at the project root.
- `titlebar.jsx`, `sidebar.jsx`, `panes.jsx`, `statusbar.jsx`, `settings.jsx`, `palette.jsx` — each component as a standalone Babel-loaded file. Components attach themselves to `window` so they're shareable across files.

## What's faked

- Terminal output is static markup, not a real PTY — but the line styling (`.t-prompt / .t-cmd / .t-info / .t-success / .t-error / .t-warn / .t-dim / .t-agent`) matches the source app's classes and colors.
- Resizers don't drag (yet). The 2×2 grid is composed with flex so widths/heights are nominal.
- Sidebar tabs (sessions / agents / worktrees / env) only render the `sessions` view.
- `⌘K` and `⌘,` are wired up; `Esc` closes overlays.

## Adding a new screen

1. Use `<div class="tm-app">` as the root — it owns the grid + the CRT ambient layers (radial glow + scanlines).
2. Pull design tokens from `/colors_and_type.css` (already imported in `index.html`).
3. Borrow components by including the relevant `.jsx` file via `<script type="text/babel" src="...">`.
4. For status colors, use the canonical names: `status-running` (sage, pulsing), `status-agent` (violet), `status-error` (rust), `status-idle` (azure), `status-stopped` (muted).
5. Icons come from `/assets/icons/icons.svg` — fetch + inline at boot, then `<use href="#name" />`. Available: `brand`, `search`, `terminal`, `agent`, `worktree`, `session`, `env`, `plus`, `close`, `sidebar`, `fullscreen`, `split-right`, `split-down`, `grid`, `balance`, `settings`, `gear`, `shell`, `kbd`, `collapse`, `agent-badge`.

## Don'ts

- No emoji. No Lucide / Heroicons. No PNG icons. Hand-rolled inline SVG only.
- No sans-serif fallback. Body, labels, brand, terminal output — all monospace.
- No glassmorphism. Modal scrim uses backdrop-filter blur; nothing else does.
- Don't introduce new accent colors — use the amber ramp + the four semantic accents (sage / rust / ember / azure / violet).
- Don't add drop shadows to cards. The active-pane treatment is darkening (`inset` + dark outer), not lifting.
