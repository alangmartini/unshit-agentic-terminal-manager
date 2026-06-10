import React from 'react';
import {bg, border, fg, fontMono} from '../theme';

export const Keycap: React.FC<{children: React.ReactNode; scale?: number}> = ({
  children,
  scale = 1,
}) => (
  <span
    style={{
      fontFamily: fontMono,
      fontWeight: 500,
      fontSize: 10 * scale,
      lineHeight: 1,
      color: fg.secondary,
      background: bg.base,
      border: `1px solid ${border.hair}`,
      padding: `${2 * scale}px ${6 * scale}px`,
      borderRadius: 2 * scale,
      margin: `0 ${2 * scale}px`,
      display: 'inline-block',
    }}
  >
    {children}
  </span>
);
