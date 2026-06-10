# terminal.mgr — demo video

Remotion project for the product demo. Recreates the app UI in React using the
design tokens from `../Unshit Terminal Design System/colors_and_type.css`
(mirrored in `src/theme.ts` — that CSS file stays the source of truth).

## Scenes

- `Intro` — brand reveal + typed tagline on the CRT background
- `CommandPalette` — app shell, `ctrl shift p` callout, palette opens, `split`
  typed with live filtering, Enter flash, pane splits

`Demo` chains all scenes (1920×1080 @ 30fps).

## Commands

```
npm run dev      # Remotion Studio (live preview, scrubbing)
npm run render   # render Demo -> out/demo.mp4
npm run still    # render a single frame -> out/frame.png
```

## Conventions

- All animation is frame-driven (`useCurrentFrame`), never CSS animations —
  Remotion renders deterministically frame by frame.
- Motion follows the design system: 120/200/320ms equivalents, ease-out lifts,
  hard on/off cursor blink (steps(2), ~1.1s), no bounce.
- The app shell renders at native design density (1280×800, 12px type) and is
  scaled up inside the 1080p composition so token pixel values stay faithful.
- Design don'ts apply here too: no emoji, mono everywhere, no glassmorphism,
  amber ramp + sage/rust/ember/azure/violet only.
