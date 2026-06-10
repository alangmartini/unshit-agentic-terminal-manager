import React from 'react';
import {AbsoluteFill, interpolate, useCurrentFrame} from 'remotion';
import {Crt} from '../components/Crt';
import {TypeText} from '../components/TypeText';
import {amber, fg, fontMono} from '../theme';

// Brand reveal: ◆ terminal.mgr + typed tagline.
export const Intro: React.FC = () => {
  const frame = useCurrentFrame();

  // ease-out lift, 200ms feel: fade + 12px rise (modal-entry motion from the system)
  const lift = interpolate(frame, [0, 10], [12, 0], {extrapolateRight: 'clamp'});
  const fade = interpolate(frame, [0, 10], [0, 1], {extrapolateRight: 'clamp'});
  const glowPulse = 0.35 + 0.15 * Math.sin(frame / 18);

  return (
    <Crt>
      <AbsoluteFill style={{justifyContent: 'center', alignItems: 'center', fontFamily: fontMono}}>
        <div style={{opacity: fade, transform: `translateY(${lift}px)`, textAlign: 'center'}}>
          <div
            style={{
              fontSize: 72,
              fontWeight: 600,
              color: fg.primary,
              textShadow: `0 0 24px rgba(212, 163, 72, ${glowPulse})`,
            }}
          >
            <span style={{color: amber[300]}}>◆</span> terminal.mgr
          </div>
          <div style={{fontSize: 24, color: fg.secondary, marginTop: 28, minHeight: 36}}>
            {frame > 20 && (
              <TypeText
                text="a terminal manager that isn't another shitty electron app"
                startFrame={20}
                charFrames={1}
              />
            )}
          </div>
          <div
            style={{
              fontSize: 16,
              color: fg.tertiary,
              marginTop: 36,
              opacity: interpolate(frame, [95, 110], [0, 1], {
                extrapolateLeft: 'clamp',
                extrapolateRight: 'clamp',
              }),
            }}
          >
            rust · gpu-rendered · zero chromium
          </div>
        </div>
      </AbsoluteFill>
    </Crt>
  );
};
