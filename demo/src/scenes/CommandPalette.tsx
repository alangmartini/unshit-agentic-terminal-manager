import React from 'react';
import {AbsoluteFill, interpolate, useCurrentFrame} from 'remotion';
import {AppShell, TerminalLines, TermLine} from '../components/AppShell';
import {Crt} from '../components/Crt';
import {Keycap} from '../components/Keycap';
import {typedLength} from '../components/TypeText';
import {accent, amber, bg, border, fg, fontMono, shadow} from '../theme';

// Timeline (30fps):
//   0-30    app shell with terminal output, idle
//   30      Ctrl+Shift+P keycap callout appears
//   45      palette opens (fade + lift)
//   55-75   "split" typed into the input, results filter
//   90      active item flashes (Enter), palette closes at 100
//   100-end pane splits into two

const PALETTE_OPEN = 45;
const TYPE_START = 55;
const TYPE_CHAR_FRAMES = 4;
const QUERY = 'split';
const ENTER = 95;
const PALETTE_CLOSE = 105;

type Item = {icon: string; label: string; hint?: string; kbd?: string[]};

const ALL_ITEMS: Item[] = [
  {icon: '⊞', label: 'Split pane right', kbd: ['ctrl', 'd']},
  {icon: '⊟', label: 'Split pane down', kbd: ['ctrl', 'shift', 'd']},
  {icon: '▦', label: 'Arrange grid 2×2', hint: 'layout'},
  {icon: '>', label: 'New terminal', kbd: ['ctrl', 't']},
  {icon: '◆', label: 'Quick prompt: spawn agent', kbd: ['ctrl', 'shift', 'q']},
  {icon: '⚙', label: 'Open settings', kbd: ['ctrl', ',']},
];

const matches = (item: Item, q: string) =>
  q === '' || item.label.toLowerCase().includes(q.toLowerCase());

const TERM_LINES: TermLine[] = [
  {kind: 'prompt', text: 'go run main.go --port 4040 --watch'},
  {kind: 'dim', text: '[14:32:07] compiling…'},
  {kind: 'success', text: '✓ build ok (412ms)'},
  {kind: 'info', text: '→ listening on http://localhost:4040'},
  {kind: 'plain', text: 'watching 142 files for changes'},
];

const SECOND_PANE: TermLine[] = [
  {kind: 'prompt', text: ''},
];

const PaletteOverlay: React.FC<{frame: number}> = ({frame}) => {
  if (frame < PALETTE_OPEN || frame > PALETTE_CLOSE) return null;

  const t = interpolate(frame, [PALETTE_OPEN, PALETTE_OPEN + 5], [0, 1], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
  });
  const closing = interpolate(frame, [PALETTE_CLOSE - 4, PALETTE_CLOSE], [1, 0], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
  });
  const opacity = Math.min(t, closing);
  const liftY = (1 - t) * 12;

  const typed = QUERY.slice(0, typedLength(frame, TYPE_START, TYPE_CHAR_FRAMES));
  const visible = ALL_ITEMS.filter((i) => matches(i, typed));
  const flash = frame >= ENTER && Math.floor((frame - ENTER) / 3) % 2 === 0;
  const blinkOn = Math.floor(frame / 16) % 2 === 0;

  return (
    <div
      style={{
        position: 'absolute',
        inset: 0,
        zIndex: 60,
        background: 'rgba(10, 8, 6, 0.6)',
        backdropFilter: 'blur(4px)',
        display: 'flex',
        alignItems: 'flex-start',
        justifyContent: 'center',
        paddingTop: 110,
        opacity,
      }}
    >
      <div
        style={{
          width: 480,
          background: bg.elevated,
          border: `1px solid ${border.default}`,
          borderRadius: 6,
          boxShadow: `${shadow.lg}, 0 0 30px rgba(212, 163, 72, 0.08)`,
          overflow: 'hidden',
          transform: `translateY(${liftY}px)`,
          fontFamily: fontMono,
        }}
      >
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            padding: '10px 14px',
            borderBottom: `1px solid ${border.hair}`,
          }}
        >
          <span style={{color: amber[300], fontWeight: 600}}>❯</span>
          <span style={{flex: 1, fontSize: 12, color: fg.primary}}>
            {typed}
            <span
              style={{
                display: 'inline-block',
                width: 7,
                height: 14,
                verticalAlign: 'text-bottom',
                background: blinkOn ? amber[300] : 'transparent',
              }}
            />
          </span>
          <Keycap>esc</Keycap>
        </div>
        <div style={{padding: '6px 0'}}>
          <div
            style={{
              fontSize: 10,
              letterSpacing: '0.12em',
              textTransform: 'uppercase',
              color: fg.tertiary,
              padding: '4px 14px',
            }}
          >
            actions
          </div>
          {visible.map((item, i) => {
            const active = i === 0;
            return (
              <div
                key={item.label}
                style={{
                  display: 'grid',
                  gridTemplateColumns: '14px 1fr auto',
                  gap: 10,
                  alignItems: 'center',
                  padding: '6px 14px',
                  background:
                    active && flash
                      ? 'rgba(212, 163, 72, 0.28)'
                      : active
                        ? 'rgba(212, 163, 72, 0.12)'
                        : 'transparent',
                  boxShadow: active ? `inset 2px 0 0 ${amber[300]}` : undefined,
                }}
              >
                <span style={{color: amber[300], fontSize: 11}}>{item.icon}</span>
                <span style={{fontSize: 11, fontWeight: 500, color: fg.primary}}>{item.label}</span>
                <span style={{display: 'flex', gap: 1, alignItems: 'center'}}>
                  {item.hint && <span style={{fontSize: 10, color: fg.tertiary}}>{item.hint}</span>}
                  {item.kbd?.map((k) => <Keycap key={k}>{k}</Keycap>)}
                </span>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
};

export const CommandPalette: React.FC = () => {
  const frame = useCurrentFrame();

  const split = frame > PALETTE_CLOSE;
  const splitT = interpolate(frame, [PALETTE_CLOSE, PALETTE_CLOSE + 8], [0, 1], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
  });

  const calloutOpacity = interpolate(frame, [30, 38, PALETTE_OPEN + 10, PALETTE_OPEN + 18], [0, 1, 1, 0], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
  });

  return (
    <Crt>
      <AbsoluteFill style={{justifyContent: 'center', alignItems: 'center'}}>
        <div style={{transform: 'scale(1.25)'}}>
          <AppShell overlay={<PaletteOverlay frame={frame} />}>
            <div style={{display: 'flex', height: '100%'}}>
              <div style={{flex: 1, minWidth: 0}}>
                <TerminalLines lines={TERM_LINES} />
              </div>
              {split && (
                <div
                  style={{
                    flex: splitT,
                    minWidth: 0,
                    borderLeft: `1px solid ${border.default}`,
                    boxShadow: `inset 0 0 0 1px rgba(212, 163, 72, ${0.12 * splitT})`,
                    opacity: splitT,
                  }}
                >
                  <TerminalLines lines={SECOND_PANE} />
                </div>
              )}
            </div>
          </AppShell>
        </div>

        {/* keystroke callout */}
        <div
          style={{
            position: 'absolute',
            bottom: 60,
            opacity: calloutOpacity,
            fontFamily: fontMono,
            fontSize: 14,
            color: fg.secondary,
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            background: bg.void,
            border: `1px solid ${border.soft}`,
            borderRadius: 3,
            padding: '10px 18px',
          }}
        >
          <Keycap scale={1.6}>ctrl</Keycap>
          <Keycap scale={1.6}>shift</Keycap>
          <Keycap scale={1.6}>p</Keycap>
          <span style={{marginLeft: 8, color: accent.azure}}>command palette</span>
        </div>
      </AbsoluteFill>
    </Crt>
  );
};
