import React from 'react';
import {Composition, Series} from 'remotion';
import {CommandPalette} from './scenes/CommandPalette';
import {Intro} from './scenes/Intro';

const FPS = 30;
const INTRO_FRAMES = 130;
const PALETTE_FRAMES = 160;

const Demo: React.FC = () => (
  <Series>
    <Series.Sequence durationInFrames={INTRO_FRAMES}>
      <Intro />
    </Series.Sequence>
    <Series.Sequence durationInFrames={PALETTE_FRAMES}>
      <CommandPalette />
    </Series.Sequence>
  </Series>
);

// JetBrains Mono, loaded the same way the design system does.
const Fonts: React.FC<{children: React.ReactNode}> = ({children}) => (
  <>
    <style>
      {`@import url("https://fonts.googleapis.com/css2?family=JetBrains+Mono:ital,wght@0,400;0,500;0,600;0,700;1,400&display=swap");`}
    </style>
    {children}
  </>
);

export const RemotionRoot: React.FC = () => (
  <>
    <Composition
      id="Demo"
      component={() => (
        <Fonts>
          <Demo />
        </Fonts>
      )}
      durationInFrames={INTRO_FRAMES + PALETTE_FRAMES}
      fps={FPS}
      width={1920}
      height={1080}
    />
    <Composition
      id="Intro"
      component={() => (
        <Fonts>
          <Intro />
        </Fonts>
      )}
      durationInFrames={INTRO_FRAMES}
      fps={FPS}
      width={1920}
      height={1080}
    />
    <Composition
      id="CommandPalette"
      component={() => (
        <Fonts>
          <CommandPalette />
        </Fonts>
      )}
      durationInFrames={PALETTE_FRAMES}
      fps={FPS}
      width={1920}
      height={1080}
    />
  </>
);
