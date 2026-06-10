import React from 'react';
import {AbsoluteFill} from 'remotion';
import {bg} from '../theme';

// CRT ambient layers from the design system: warm radial glow + scanlines.
// "Without it, the system loses 30% of its identity."
export const Crt: React.FC<{children?: React.ReactNode}> = ({children}) => (
  <AbsoluteFill style={{background: bg.base}}>
    <AbsoluteFill style={{zIndex: 3}}>{children}</AbsoluteFill>
    <AbsoluteFill
      style={{
        pointerEvents: 'none',
        zIndex: 4,
        background:
          'radial-gradient(ellipse 60% 40% at 85% 0%, rgba(212, 163, 72, 0.035), transparent 70%), ' +
          'radial-gradient(ellipse 80% 50% at 10% 100%, rgba(138, 96, 32, 0.025), transparent 70%)',
      }}
    />
    <AbsoluteFill
      style={{
        pointerEvents: 'none',
        zIndex: 5,
        backgroundImage:
          'repeating-linear-gradient(0deg, transparent 0, transparent 2px, rgba(0,0,0,0.12) 2px, rgba(0,0,0,0.12) 3px)',
        opacity: 0.22,
        mixBlendMode: 'multiply',
      }}
    />
  </AbsoluteFill>
);
