// Design tokens mirrored from `Unshit Terminal Design System/colors_and_type.css`.
// Keep in sync with that file — it is the source of truth.

export const bg = {
  void: '#14110c',
  base: '#1c1812',
  subtle: '#221d16',
  elevated: '#29231a',
  hover: '#342c20',
  selected: '#3e3425',
  overlay: 'rgba(12, 10, 7, 0.72)',
} as const;

export const border = {
  hair: '#241e15',
  soft: '#342b1e',
  default: '#4a3e2a',
  strong: '#6a5939',
  focus: '#d4a348',
} as const;

export const fg = {
  primary: '#ebdcb6',
  secondary: '#b8a275',
  tertiary: '#857149',
  muted: '#4e422b',
  ghost: '#2f2819',
} as const;

export const amber = {
  50: '#fdf1cf',
  100: '#f6d988',
  200: '#e8b955',
  300: '#d4a348',
  400: '#b8852c',
  500: '#8a6020',
  600: '#5c3f12',
} as const;

export const accent = {
  ember: '#d06e2c',
  sage: '#8ba85c',
  rust: '#c9553a',
  azure: '#6aa2ad',
  violet: '#a88bb8',
} as const;

export const fontMono =
  "'JetBrains Mono', 'Berkeley Mono', 'SF Mono', Menlo, Consolas, monospace";

export const shadow = {
  sm: '0 1px 2px rgba(0, 0, 0, 0.3)',
  md: '0 4px 14px rgba(0, 0, 0, 0.45)',
  lg: '0 14px 40px rgba(0, 0, 0, 0.65)',
} as const;

export const glow = {
  amber: '0 0 18px rgba(212, 163, 72, 0.18)',
  amberS: '0 0 8px rgba(212, 163, 72, 0.25)',
  sage: '0 0 6px rgba(139, 168, 92, 0.45)',
  rust: '0 0 6px rgba(201, 85, 58, 0.45)',
} as const;
