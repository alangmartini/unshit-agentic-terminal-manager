import React from 'react';
import {accent, amber, bg, border, fg, fontMono} from '../theme';
import {Keycap} from './Keycap';

// Simplified recreation of the app chrome from ui_kits/terminal_manager.
// Rendered at native design density (1280x800) and scaled up by the scene.

const TITLEBAR_H = 34;
const SIDEBAR_W = 252;
const TABBAR_H = 38;
const STATUSBAR_H = 24;

const Row: React.FC<{children: React.ReactNode; style?: React.CSSProperties}> = ({
  children,
  style,
}) => <div style={{display: 'flex', alignItems: 'center', ...style}}>{children}</div>;

const StatusDot: React.FC<{color: string; glowColor?: string}> = ({color, glowColor}) => (
  <span
    style={{
      width: 6,
      height: 6,
      borderRadius: 3,
      background: color,
      boxShadow: glowColor ? `0 0 6px ${glowColor}` : undefined,
      display: 'inline-block',
      flexShrink: 0,
    }}
  />
);

const SidebarRow: React.FC<{
  glyph: string;
  label: string;
  hint?: string;
  active?: boolean;
  status?: 'running' | 'agent' | 'idle';
}> = ({glyph, label, hint, active, status}) => (
  <Row
    style={{
      padding: '4px 12px',
      gap: 8,
      fontSize: 11,
      color: active ? amber[100] : fg.secondary,
      background: active ? bg.elevated : 'transparent',
      boxShadow: active ? `inset 2px 0 0 ${amber[300]}` : undefined,
    }}
  >
    <span style={{color: fg.tertiary}}>{glyph}</span>
    <span style={{flex: 1, overflow: 'hidden', whiteSpace: 'nowrap'}}>{label}</span>
    {status === 'running' && <StatusDot color={accent.sage} glowColor="rgba(139,168,92,0.45)" />}
    {status === 'agent' && <StatusDot color={accent.violet} />}
    {status === 'idle' && <StatusDot color={accent.azure} />}
    {hint && <span style={{fontSize: 10, color: fg.tertiary}}>{hint}</span>}
  </Row>
);

const Tab: React.FC<{label: string; active?: boolean}> = ({label, active}) => (
  <Row
    style={{
      padding: '0 14px',
      gap: 8,
      fontSize: 11,
      height: '100%',
      color: active ? amber[100] : fg.tertiary,
      background: active ? bg.base : 'transparent',
      borderRight: `1px solid ${border.hair}`,
      boxShadow: active ? `inset 0 2px 0 ${amber[300]}` : undefined,
      textShadow: active ? '0 0 8px rgba(212,163,72,0.35)' : undefined,
    }}
  >
    <span>{label}</span>
    <span style={{color: fg.muted}}>×</span>
  </Row>
);

export type TermLine = {
  kind: 'prompt' | 'cmd' | 'info' | 'success' | 'error' | 'dim' | 'agent' | 'plain';
  text: string;
};

const lineColor: Record<TermLine['kind'], string> = {
  prompt: amber[300],
  cmd: fg.primary,
  info: accent.azure,
  success: accent.sage,
  error: accent.rust,
  dim: fg.tertiary,
  agent: accent.violet,
  plain: fg.secondary,
};

export const TerminalLines: React.FC<{lines: TermLine[]}> = ({lines}) => (
  <div style={{fontFamily: fontMono, fontSize: 12, lineHeight: 1.55, padding: 14}}>
    {lines.map((l, i) => (
      <div key={i} style={{color: lineColor[l.kind], whiteSpace: 'pre'}}>
        {l.kind === 'prompt' ? (
          <>
            <span style={{color: amber[300], fontWeight: 600, textShadow: '0 0 8px rgba(212,163,72,0.4)'}}>
              ❯{' '}
            </span>
            <span style={{color: fg.primary}}>{l.text}</span>
          </>
        ) : (
          l.text
        )}
      </div>
    ))}
  </div>
);

export const AppShell: React.FC<{
  children?: React.ReactNode; // pane content
  overlay?: React.ReactNode; // e.g. command palette
}> = ({children, overlay}) => (
  <div
    style={{
      width: 1280,
      height: 800,
      display: 'grid',
      gridTemplateRows: `${TITLEBAR_H}px ${TABBAR_H}px 1fr ${STATUSBAR_H}px`,
      gridTemplateColumns: `${SIDEBAR_W}px 1fr`,
      background: bg.base,
      color: fg.primary,
      fontFamily: fontMono,
      fontSize: 12,
      overflow: 'hidden',
      position: 'relative',
      border: `1px solid ${border.soft}`,
      borderRadius: 6,
    }}
  >
    {/* titlebar */}
    <Row
      style={{
        gridColumn: '1 / -1',
        background: bg.void,
        borderBottom: `1px solid ${border.hair}`,
        padding: '0 12px',
        gap: 10,
      }}
    >
      <span style={{color: amber[300], fontWeight: 600}}>◆</span>
      <span style={{fontSize: 11, color: fg.secondary}}>terminal.mgr</span>
      <span style={{flex: 1}} />
      <span style={{fontSize: 10, color: fg.tertiary}}>
        search <Keycap>ctrl</Keycap>
        <Keycap>shift</Keycap>
        <Keycap>p</Keycap>
      </span>
    </Row>

    {/* sidebar */}
    <div
      style={{
        gridRow: '2 / 4',
        background: bg.subtle,
        borderRight: `1px solid ${border.hair}`,
        paddingTop: 8,
      }}
    >
      <div
        style={{
          fontSize: 10,
          letterSpacing: '0.12em',
          textTransform: 'uppercase',
          color: fg.tertiary,
          padding: '4px 12px',
        }}
      >
        workspaces
      </div>
      <SidebarRow glyph="▾" label="main" hint="(main)" active />
      <SidebarRow glyph="├" label="dashboard-dev" status="running" />
      <SidebarRow glyph="├" label="claude · refactor-userlist" status="agent" />
      <SidebarRow glyph="└" label="logs" status="idle" />
      <SidebarRow glyph="▸" label="api" hint="(fix/pdf-export)" />
      <SidebarRow glyph="▸" label="infra" />
      <SidebarRow glyph="▸" label="scratch" />
    </div>

    {/* tab strip */}
    <Row
      style={{
        background: bg.subtle,
        borderBottom: `1px solid ${border.hair}`,
        alignItems: 'stretch',
      }}
    >
      <Tab label="dashboard-dev" active />
      <Tab label="api-server" />
      <Tab label="claude · agent" />
      <Row style={{padding: '0 10px', color: fg.tertiary}}>+</Row>
    </Row>

    {/* pane */}
    <div style={{background: bg.base, position: 'relative', overflow: 'hidden'}}>{children}</div>

    {/* statusbar */}
    <Row
      style={{
        gridColumn: '2',
        background: bg.void,
        borderTop: `1px solid ${border.hair}`,
        padding: '0 12px',
        gap: 14,
        fontSize: 10,
        color: fg.tertiary,
      }}
    >
      <span style={{color: accent.sage}}>● 3 running</span>
      <span style={{color: accent.violet}}>◆ 1 agent</span>
      <span style={{flex: 1}} />
      <span>unshit-ptyd connected</span>
      <span style={{color: accent.azure}}>~/main/dashboard</span>
    </Row>

    {overlay}
  </div>
);
