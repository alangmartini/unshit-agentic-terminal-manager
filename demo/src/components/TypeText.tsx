import React from 'react';
import {useCurrentFrame} from 'remotion';
import {amber} from '../theme';

// Frame-deterministic typewriter. Types one char every `charFrames` frames
// starting at `startFrame`. Cursor blinks hard on/off (steps(2), no fade),
// matching the design system's 1.1s terminal cursor.
export const TypeText: React.FC<{
  text: string;
  startFrame?: number;
  charFrames?: number;
  cursor?: boolean;
  cursorColor?: string;
}> = ({text, startFrame = 0, charFrames = 2, cursor = true, cursorColor = amber[300]}) => {
  const frame = useCurrentFrame();
  const typed = Math.max(0, Math.min(text.length, Math.floor((frame - startFrame) / charFrames)));
  const blinkOn = Math.floor(frame / 16) % 2 === 0; // ~1.1s period at 30fps
  return (
    <span>
      {text.slice(0, typed)}
      {cursor && (
        <span
          style={{
            display: 'inline-block',
            width: '0.6em',
            height: '1.1em',
            verticalAlign: 'text-bottom',
            background: blinkOn ? cursorColor : 'transparent',
            boxShadow: blinkOn ? `0 0 8px rgba(212, 163, 72, 0.25)` : 'none',
          }}
        />
      )}
    </span>
  );
};

export const typedLength = (frame: number, startFrame: number, charFrames: number) =>
  Math.max(0, Math.floor((frame - startFrame) / charFrames));
