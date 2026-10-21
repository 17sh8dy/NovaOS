import { useEffect, useRef } from 'react';
import { bitColor, type BitsData } from '../../settings/bits';
import { hexToRgbTriplet } from '../../settings/schema';
import {
  makeSparks,
  makeSwarm,
  sparkLine,
  stepSwarm,
  type Dot,
  type Pt,
  type Spark,
} from './engine';

/** The real `#rrggbb` for a stored effect color, read from the desktop's CSS (so Accent follows the theme). */
export function resolveColor(el: Element, c: string): string {
  if (/^#[0-9a-f]{6}$/i.test(c)) return c;
  const accent = getComputedStyle(el).getPropertyValue('--accent').trim();
  return /^#[0-9a-f]{6}$/i.test(accent) ? accent : '#8b5cf6';
}

const rgba = (hex: string, a: number) => `rgb(${hexToRgbTriplet(hex)} / ${a})`;

/**
 * Click Spark, Glow Cursor and Swarm Cursor, drawn on one transparent canvas above everything
 * (but below NovaOS's own pointer). It ignores the mouse completely, so it can never block a
 * click. The loop only runs while something is animating.
 */
export function BitsOverlay({ bits }: { bits: BitsData }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const cfg = useRef(bits);
  cfg.current = bits;
  const redraw = useRef<() => void>(() => undefined);
  const wanted = bits.clickSpark.enabled || bits.glowCursor.enabled || bits.swarmCursor.enabled;

  useEffect(() => {
    const el = canvas.current;
    const ctx = el?.getContext?.('2d') ?? null;
    if (!wanted || !el || !ctx) return;

    let dpr = 1;
    const fit = () => {
      dpr = Math.min(window.devicePixelRatio || 1, 2);
      el.width = Math.max(1, Math.round(window.innerWidth * dpr));
      el.height = Math.max(1, Math.round(window.innerHeight * dpr));
    };
    fit();

    let pointer: Pt | null = null;
    let sparks: Spark[] = [];
    let swarm: Dot[] = [];
    let raf = 0;
    let last = 0;

    const frame = () => {
      raf = 0;
      // Same clock as the sparks' birth times (a frame's own timestamp can be a hair earlier).
      const now = performance.now();
      const c = cfg.current;
      const dt = last ? (now - last) / 1000 : 0.016;
      last = now;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, window.innerWidth, window.innerHeight);
      let animating = false;

      if (c.glowCursor.enabled && pointer) {
        const g = c.glowCursor;
        const hex = resolveColor(el, bitColor(g.color));
        const r = g.size / 2;
        const grad = ctx.createRadialGradient(pointer.x, pointer.y, 0, pointer.x, pointer.y, r);
        grad.addColorStop(0, rgba(hex, g.intensity));
        grad.addColorStop(0.5, rgba(hex, g.intensity * 0.35));
        grad.addColorStop(1, rgba(hex, 0));
        ctx.fillStyle = grad;
        ctx.fillRect(pointer.x - r, pointer.y - r, r * 2, r * 2);
      }

      if (c.swarmCursor.enabled && pointer) {
        const s = c.swarmCursor;
        if (swarm.length !== s.count) swarm = makeSwarm(s.count, pointer);
        stepSwarm(swarm, pointer, dt, now / 1000);
        const hex = resolveColor(el, bitColor(s.color));
        for (const d of swarm) {
          ctx.beginPath();
          ctx.fillStyle = rgba(hex, 0.9);
          ctx.arc(d.x, d.y, s.size / 2, 0, Math.PI * 2);
          ctx.fill();
        }
        animating = true;
      } else if (swarm.length) {
        swarm = [];
      }

      if (c.clickSpark.enabled && sparks.length) {
        const s = c.clickSpark;
        const hex = resolveColor(el, bitColor(s.color));
        ctx.lineCap = 'round';
        ctx.lineWidth = 2;
        sparks = sparks.filter((sp) => now - sp.born < s.duration);
        for (const sp of sparks) {
          const l = sparkLine(sp, now, s.duration, s.length);
          if (!l) continue;
          ctx.strokeStyle = rgba(hex, l.alpha);
          ctx.beginPath();
          ctx.moveTo(l.x1, l.y1);
          ctx.lineTo(l.x2, l.y2);
          ctx.stroke();
        }
        animating = animating || sparks.length > 0;
      } else if (sparks.length) {
        sparks = [];
      }

      if (animating) schedule();
    };

    const schedule = () => {
      if (!raf && !document.hidden) raf = requestAnimationFrame(frame);
    };

    const onMove = (e: PointerEvent) => {
      if (e.pointerType === 'touch') return;
      pointer = { x: e.clientX, y: e.clientY };
      schedule();
    };
    const onDown = (e: PointerEvent) => {
      if (e.pointerType === 'touch' || !cfg.current.clickSpark.enabled) return;
      const s = cfg.current.clickSpark;
      sparks = [
        ...sparks.slice(-60),
        ...makeSparks(e.clientX, e.clientY, s.count, performance.now()),
      ];
      pointer = { x: e.clientX, y: e.clientY };
      schedule();
    };
    const onLeave = () => {
      pointer = null;
      schedule();
    };
    const onVisible = () => {
      if (!document.hidden) {
        last = 0;
        schedule();
      }
    };

    redraw.current = schedule;
    window.addEventListener('pointermove', onMove, { capture: true, passive: true });
    window.addEventListener('pointerdown', onDown, { capture: true, passive: true });
    window.addEventListener('resize', fit);
    document.documentElement.addEventListener('mouseleave', onLeave);
    document.addEventListener('visibilitychange', onVisible);
    schedule();
    return () => {
      if (raf) cancelAnimationFrame(raf);
      window.removeEventListener('pointermove', onMove, true);
      window.removeEventListener('pointerdown', onDown, true);
      window.removeEventListener('resize', fit);
      document.documentElement.removeEventListener('mouseleave', onLeave);
      document.removeEventListener('visibilitychange', onVisible);
      ctx.clearRect(0, 0, el.width, el.height);
    };
  }, [wanted]);

  // A setting changed under a still mouse: draw once so it shows (or clears) right away.
  useEffect(() => redraw.current(), [bits]);

  if (!wanted) return null;
  return (
    <canvas
      ref={canvas}
      data-testid="bits-overlay"
      aria-hidden="true"
      className="pointer-events-none fixed top-0 left-0 h-screen w-screen"
      style={{ zIndex: 2147482000 }}
    />
  );
}
