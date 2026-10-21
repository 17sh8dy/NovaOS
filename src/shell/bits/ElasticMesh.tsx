import { useEffect, useRef } from 'react';
import { bitColor, type BitsData } from '../../settings/bits';
import { hexToRgbTriplet } from '../../settings/schema';
import { Mesh, type Pt } from './engine';
import { resolveColor } from './BitsOverlay';

/**
 * Elastic Mesh: a stretchy grid drawn right above the wallpaper and below every window and icon.
 * It ignores the mouse completely. The animation loop sleeps whenever the mesh is at rest.
 */
export function ElasticMesh({ config }: { config: BitsData['elasticMesh'] }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const cfg = useRef(config);
  cfg.current = config;
  const redraw = useRef<() => void>(() => undefined);
  const { enabled, spacing } = config;

  useEffect(() => {
    const el = canvas.current;
    const ctx = el?.getContext?.('2d') ?? null;
    if (!enabled || !el || !ctx) return;

    let dpr = 1;
    let mesh = new Mesh(window.innerWidth, window.innerHeight, spacing);
    const fit = () => {
      dpr = Math.min(window.devicePixelRatio || 1, 2);
      el.width = Math.max(1, Math.round(window.innerWidth * dpr));
      el.height = Math.max(1, Math.round(window.innerHeight * dpr));
      mesh = new Mesh(window.innerWidth, window.innerHeight, cfg.current.spacing);
    };
    fit();

    let pointer: Pt | null = null;
    let raf = 0;
    let last = 0;

    const draw = () => {
      const c = cfg.current;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, window.innerWidth, window.innerHeight);
      const hex = resolveColor(el, bitColor(c.color));
      ctx.strokeStyle = `rgb(${hexToRgbTriplet(hex)} / ${c.opacity})`;
      ctx.lineWidth = 1;
      ctx.beginPath();
      const { cols, rows } = mesh;
      const px = (i: number) => mesh.rx[i] + mesh.dx[i];
      const py = (i: number) => mesh.ry[i] + mesh.dy[i];
      for (let r = 0; r < rows; r++) {
        for (let col = 0; col < cols; col++) {
          const i = r * cols + col;
          if (col + 1 < cols) {
            ctx.moveTo(px(i), py(i));
            ctx.lineTo(px(i + 1), py(i + 1));
          }
          if (r + 1 < rows) {
            ctx.moveTo(px(i), py(i));
            ctx.lineTo(px(i + cols), py(i + cols));
          }
        }
      }
      ctx.stroke();
    };

    const frame = (now: number) => {
      raf = 0;
      const dt = last ? (now - last) / 1000 : 0.016;
      last = now;
      const energy = mesh.step(pointer, cfg.current.strength, dt);
      draw();
      if (energy > 0) schedule();
      else last = 0; // at rest: sleep until the pointer moves again
    };
    const schedule = () => {
      if (!raf && !document.hidden) raf = requestAnimationFrame(frame);
    };

    const onMove = (e: PointerEvent) => {
      if (e.pointerType === 'touch') return;
      pointer = { x: e.clientX, y: e.clientY };
      schedule();
    };
    const onLeave = () => {
      pointer = null;
      schedule();
    };
    window.addEventListener('pointermove', onMove, { capture: true, passive: true });
    window.addEventListener('resize', fit);
    document.documentElement.addEventListener('mouseleave', onLeave);
    draw();
    redraw.current = draw;
    return () => {
      redraw.current = () => undefined;
      if (raf) cancelAnimationFrame(raf);
      window.removeEventListener('pointermove', onMove, true);
      window.removeEventListener('resize', fit);
      document.documentElement.removeEventListener('mouseleave', onLeave);
    };
  }, [enabled, spacing]);

  // Color / strength / opacity changed: redraw so the change shows under a still mouse.
  useEffect(() => redraw.current(), [config.color, config.opacity]);

  if (!enabled) return null;
  return (
    <canvas
      ref={canvas}
      data-testid="elastic-mesh"
      aria-hidden="true"
      className="pointer-events-none absolute inset-0 h-full w-full"
    />
  );
}
