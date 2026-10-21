import { useMemo, type CSSProperties } from 'react';
import { activeBits, bitColor, type BitsData } from '../../settings/bits';
import { useSettings } from '../../settings/store';

/** The React Bits settings that should really run: Reduce motion and High contrast always win. */
export function useActiveBits(): BitsData {
  const bits = useSettings((s) => s.data.bits);
  const reduceMotion = useSettings((s) => s.data.accessibility.reduceMotion);
  const highContrast = useSettings((s) => s.data.accessibility.highContrast);
  return useMemo(
    () => activeBits(bits, { reduceMotion, highContrast }),
    [bits, reduceMotion, highContrast],
  );
}

/** CSS variables the effects' styles read. Set on the desktop root so every window inherits them. */
export function bitsVars(b: BitsData): CSSProperties {
  return {
    '--bit-gray': b.chromaGrid.gray,
    '--bit-shiny-dur': `${b.shinyText.period}s`,
    '--bit-echo': bitColor(b.echoText.color),
    '--bit-echo-d': `${b.echoText.spread}px`,
    '--bit-fold-speed': b.foldText.speed,
    '--bit-list-stagger': `${b.animatedList.stagger}ms`,
    '--bit-bento': bitColor(b.magicBento.color),
    '--bit-bento-r': `${b.magicBento.radius}px`,
    '--bit-glow': bitColor(b.borderGlow.color),
  } as CSSProperties;
}

/** Words on the desktop root that switch on the pure-CSS effects (see styles.css). */
export function bitsAttr(b: BitsData): string {
  const on: string[] = [];
  if (b.chromaGrid.enabled) on.push('chroma');
  if (b.magicBento.enabled) on.push('bento');
  if (b.borderGlow.enabled) on.push('glow');
  return on.join(' ');
}

/** For list items: `fx(i)` gives the class and delay that make item `i` glide in, or nothing when off. */
export function useListFx(): (index: number) => { className?: string; style?: CSSProperties } {
  const { animatedList } = useActiveBits();
  return (index) =>
    animatedList.enabled
      ? {
          className: `bit-li bit-li-${animatedList.style}`,
          style: { '--i': Math.min(index, 14) } as CSSProperties,
        }
      : {};
}
