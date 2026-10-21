import { useEffect } from 'react';
import type { BitsData } from '../../settings/bits';
import {
  chromaAmount,
  distToRect,
  dockScale,
  falloff,
  magnetShift,
  tiltFor,
  type Pt,
} from './engine';

/**
 * The effects that act on real elements: Magnet, Dock, Chroma Grid, Magic Bento and Border Glow.
 * Elements opt in with `data-bit="magnet dock chroma bento glow"` (any of those words); nothing
 * happens to an element unless its effect is on. One pointer listener does all the work, reading
 * every position first and writing styles after, once per frame.
 *
 * Everything it sets is inline style (`translate`, `scale`, CSS variables), and all of it is
 * removed again when an effect is turned off or this component goes away.
 */

const STYLE_PROPS = ['translate', 'scale', 'z-index', 'transform'];
const VARS = ['--chroma', '--mx', '--my', '--glow', '--gx', '--gy', '--gglow'];

function clearElement(el: HTMLElement): void {
  for (const p of STYLE_PROPS) el.style.removeProperty(p);
  for (const v of VARS) el.style.removeProperty(v);
}

export function BitsDom({ bits }: { bits: BitsData }) {
  const { magnet, dock, chromaGrid, magicBento, borderGlow } = bits;
  const any =
    magnet.enabled ||
    dock.enabled ||
    chromaGrid.enabled ||
    magicBento.enabled ||
    borderGlow.enabled;

  useEffect(() => {
    if (!any) return;
    let pointer: Pt | null = null;
    let raf = 0;
    let touched = new Set<HTMLElement>();
    // How far Magnet has already moved each element, so its true resting center can be recovered.
    const shifted = new WeakMap<HTMLElement, [number, number]>();

    const run = () => {
      raf = 0;
      const elements = [...document.querySelectorAll<HTMLElement>('[data-bit]')];
      // Pass 1: read every rectangle (no writes in between, so the page lays out once).
      const read = elements.map((el) => ({
        el,
        tokens: (el.dataset.bit ?? '').split(/\s+/),
        r: el.getBoundingClientRect(),
      }));
      // Pass 2: write.
      const nowTouched = new Set<HTMLElement>();
      for (const { el, tokens, r } of read) {
        const has = (t: string) => tokens.includes(t);
        // The rectangle includes any Magnet shift already applied; take it back out so the pull
        // is measured from where the element really sits and doesn't feed back on itself.
        const [ox, oy] = shifted.get(el) ?? [0, 0];
        const center: Pt = { x: r.left + r.width / 2 - ox, y: r.top + r.height / 2 - oy };
        let wrote = false;

        if (pointer && magnet.enabled && has('magnet')) {
          const [sx, sy] = magnetShift(center, pointer, magnet.radius, magnet.strength);
          shifted.set(el, [sx, sy]);
          if (sx === 0 && sy === 0) el.style.removeProperty('translate');
          else el.style.setProperty('translate', `${sx.toFixed(1)}px ${sy.toFixed(1)}px`);
          wrote = true;
        }
        if (pointer && dock.enabled && has('dock')) {
          const d = Math.hypot(pointer.x - center.x, pointer.y - center.y);
          const s = dockScale(d, dock.radius, dock.magnify);
          el.style.setProperty('scale', s.toFixed(3));
          if (s > 1.02) el.style.setProperty('z-index', '5');
          else el.style.removeProperty('z-index');
          wrote = true;
        }
        if (pointer && chromaGrid.enabled && has('chroma')) {
          const d = Math.hypot(pointer.x - center.x, pointer.y - center.y);
          el.style.setProperty('--chroma', chromaAmount(d, chromaGrid.radius).toFixed(3));
          wrote = true;
        }
        if (pointer && magicBento.enabled && has('bento')) {
          const near = falloff(distToRect(pointer, r), magicBento.radius);
          el.style.setProperty('--mx', `${(pointer.x - r.left).toFixed(0)}px`);
          el.style.setProperty('--my', `${(pointer.y - r.top).toFixed(0)}px`);
          el.style.setProperty('--glow', near.toFixed(3));
          if (magicBento.tilt && distToRect(pointer, r) === 0) {
            const [rx, ry] = tiltFor(pointer, r);
            el.style.setProperty(
              'transform',
              `perspective(800px) rotateX(${rx}deg) rotateY(${ry}deg)`,
            );
          } else {
            el.style.removeProperty('transform');
          }
          wrote = true;
        }
        if (pointer && borderGlow.enabled && has('glow')) {
          const near = falloff(distToRect(pointer, r), borderGlow.radius);
          el.style.setProperty('--gx', `${(pointer.x - r.left).toFixed(0)}px`);
          el.style.setProperty('--gy', `${(pointer.y - r.top).toFixed(0)}px`);
          el.style.setProperty('--gglow', near.toFixed(3));
          wrote = true;
        }
        if (wrote) nowTouched.add(el);
      }
      // Anything we styled before that is gone from the list, or left the pointer behind, resets.
      for (const el of touched) {
        if (nowTouched.has(el)) continue;
        shifted.delete(el);
        if (el.isConnected) clearElement(el);
      }
      touched = nowTouched;
    };

    const schedule = () => {
      if (!raf) raf = requestAnimationFrame(run);
    };
    const onMove = (e: PointerEvent) => {
      if (e.pointerType === 'touch') return;
      pointer = { x: e.clientX, y: e.clientY };
      schedule();
    };
    const onLeave = () => {
      pointer = null;
      schedule();
      // With no pointer, `run` writes nothing, so the loop below clears what was left behind.
    };

    window.addEventListener('pointermove', onMove, { capture: true, passive: true });
    document.documentElement.addEventListener('mouseleave', onLeave);
    return () => {
      if (raf) cancelAnimationFrame(raf);
      window.removeEventListener('pointermove', onMove, true);
      document.documentElement.removeEventListener('mouseleave', onLeave);
      document.querySelectorAll<HTMLElement>('[data-bit]').forEach(clearElement);
    };
  }, [any, magnet, dock, chromaGrid, magicBento, borderGlow]);

  return null;
}
