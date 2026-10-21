import type { TargetAndTransition } from 'motion/react';
import type { WindowAnimation } from './schema';

type Pose = TargetAndTransition;

export interface WindowMotion {
  initial: Pose;
  animate: Pose;
  transition: { duration: number };
}

const REST: Pose = { opacity: 1, scale: 1, y: 0 };

/**
 * How a window appears and minimizes for the chosen animation and speed. A window that is not
 * minimized rests at full opacity; "none" makes every change instant.
 */
export function windowMotion(
  animation: WindowAnimation,
  minimized: boolean,
  speed: number,
): WindowMotion {
  const base = 0.12 / Math.max(0.1, speed);
  switch (animation) {
    case 'none':
      return {
        initial: REST,
        animate: minimized ? { opacity: 0 } : REST,
        transition: { duration: 0 },
      };
    case 'fade':
      return {
        initial: { opacity: 0 },
        animate: minimized ? { opacity: 0 } : { opacity: 1, scale: 1, y: 0 },
        transition: { duration: base * 1.4 },
      };
    case 'slide':
      return {
        initial: { opacity: 0, y: 18 },
        animate: minimized ? { opacity: 0, y: 26 } : REST,
        transition: { duration: base * 1.6 },
      };
    default:
      return {
        initial: { opacity: 0, scale: 0.96 },
        animate: minimized ? { opacity: 0, scale: 0.9 } : REST,
        transition: { duration: base },
      };
  }
}
