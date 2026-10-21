import { describe, expect, it } from 'vitest';
import {
  Mesh,
  chromaAmount,
  distToRect,
  dockScale,
  falloff,
  magnetShift,
  makeSparks,
  makeSwarm,
  sparkLine,
  stepSwarm,
  tiltFor,
} from './engine';

const seq = (...values: number[]) => {
  let i = 0;
  return () => values[i++ % values.length];
};

describe('Click Spark', () => {
  it('makes the requested number of sparks, all starting at the click', () => {
    const s = makeSparks(100, 50, 8, 1000, seq(0.5));
    expect(s).toHaveLength(8);
    expect(s.every((p) => p.x === 100 && p.y === 50 && p.born === 1000)).toBe(true);
  });

  it('spreads them around the whole circle', () => {
    const angles = makeSparks(0, 0, 4, 0, seq(0.5)).map((s) => s.angle);
    expect(angles[1] - angles[0]).toBeCloseTo(Math.PI / 2);
    expect(angles[3] - angles[0]).toBeCloseTo((3 * Math.PI) / 2);
  });

  it('flies outward and fades, then is gone', () => {
    const [s] = makeSparks(0, 0, 1, 0, seq(0.5));
    const early = sparkLine(s, 50, 500, 14);
    const late = sparkLine(s, 400, 500, 14);
    expect(early && late).toBeTruthy();
    expect(Math.hypot(late!.x2, late!.y2)).toBeGreaterThan(Math.hypot(early!.x2, early!.y2));
    expect(late!.alpha).toBeLessThan(early!.alpha);
    expect(sparkLine(s, 500, 500, 14)).toBeNull();
    expect(sparkLine(s, 9999, 500, 14)).toBeNull();
    expect(sparkLine(s, -10, 500, 14)).toBeNull();
  });
});

describe('Swarm Cursor', () => {
  it('starts every dot at the pointer and gives each its own lag', () => {
    const dots = makeSwarm(10, { x: 5, y: 6 }, seq(0.1, 0.9, 0.5));
    expect(dots).toHaveLength(10);
    expect(dots.every((d) => d.x === 5 && d.y === 6)).toBe(true);
    expect(new Set(dots.map((d) => d.lag)).size).toBeGreaterThan(1);
  });

  it('chases the pointer and stays near it', () => {
    const dots = makeSwarm(12, { x: 0, y: 0 });
    for (let i = 0; i < 240; i++) stepSwarm(dots, { x: 500, y: 300 }, 1 / 60, i / 60);
    for (const d of dots) expect(Math.hypot(d.x - 500, d.y - 300)).toBeLessThan(50);
  });

  it('a long pause between frames never flings a dot far away', () => {
    const dots = makeSwarm(5, { x: 0, y: 0 });
    stepSwarm(dots, { x: 100, y: 100 }, 30, 0); // a 30 second hiccup
    for (const d of dots) {
      expect(Number.isFinite(d.x) && Number.isFinite(d.y)).toBe(true);
      expect(Math.hypot(d.x - 100, d.y - 100)).toBeLessThan(200);
    }
  });
});

describe('Elastic Mesh', () => {
  it('lays out a grid covering the screen', () => {
    const m = new Mesh(200, 100, 50);
    expect(m.cols).toBe(5);
    expect(m.rows).toBe(3);
    expect(m.rx[1]).toBe(50);
    expect(m.ry[m.cols]).toBe(50);
  });

  it('is at rest when nothing touches it', () => {
    const m = new Mesh(300, 200, 50);
    expect(m.step(null, 0.8, 0.016)).toBe(0);
    expect(m.dx.every((v) => v === 0)).toBe(true);
  });

  it('bends away from a nearby pointer, then springs back to flat when it leaves', () => {
    const m = new Mesh(400, 300, 40);
    const near = (m.rows >> 1) * m.cols + (m.cols >> 1);
    const p = { x: m.rx[near] + 6, y: m.ry[near] };
    let moved = 0;
    for (let i = 0; i < 20; i++) moved = m.step(p, 1, 1 / 60);
    expect(moved).toBeGreaterThan(0);
    expect(m.dx[near]).toBeLessThan(-0.5); // pushed left, away from a pointer on its right
    let energy = 1;
    for (let i = 0; i < 1200 && energy > 0; i++) energy = m.step(null, 1, 1 / 60);
    expect(energy).toBe(0);
    expect(Math.max(...m.dx.map(Math.abs))).toBeLessThan(0.5);
  });

  it('a point can never be dragged more than about a grid step', () => {
    const m = new Mesh(200, 200, 40);
    const p = { x: m.rx[12] + 1, y: m.ry[12] + 1 };
    for (let i = 0; i < 300; i++) m.step(p, 1.5, 1 / 30);
    for (let i = 0; i < m.dx.length; i++) {
      expect(Math.hypot(m.dx[i], m.dy[i])).toBeLessThanOrEqual(40 * 0.9 + 0.001);
    }
  });

  it('refuses to build a huge mesh', () => {
    const m = new Mesh(7680, 4320, 8);
    expect(m.rx.length).toBeLessThanOrEqual(Mesh.MAX_POINTS);
    expect(m.spacing).toBeGreaterThan(8);
  });
});

describe('proximity helpers', () => {
  it('distToRect is 0 inside and the straight-line gap outside', () => {
    const r = { left: 10, top: 10, right: 50, bottom: 40 };
    expect(distToRect({ x: 20, y: 20 }, r)).toBe(0);
    expect(distToRect({ x: 60, y: 20 }, r)).toBe(10);
    expect(distToRect({ x: 53, y: 44 }, r)).toBe(5);
  });

  it('falloff is 1 at the pointer, 0 at the radius and beyond, and smooth between', () => {
    expect(falloff(0, 100)).toBe(1);
    expect(falloff(100, 100)).toBe(0);
    expect(falloff(500, 100)).toBe(0);
    expect(falloff(50, 100)).toBeCloseTo(0.5);
    expect(falloff(10, 0)).toBe(0);
  });

  it('Dock grows items near the pointer only', () => {
    expect(dockScale(0, 120, 1.5)).toBeCloseTo(1.5);
    expect(dockScale(120, 120, 1.5)).toBe(1);
    expect(dockScale(999, 120, 1.5)).toBe(1);
    expect(dockScale(40, 120, 1.5)).toBeGreaterThan(dockScale(80, 120, 1.5));
  });

  it('Magnet leans toward the pointer, harder when closer, not at all when out of reach', () => {
    const from = { x: 0, y: 0 };
    const [nx] = magnetShift(from, { x: 100, y: 0 }, 200, 0.5);
    const [fx] = magnetShift(from, { x: 150, y: 0 }, 200, 0.5);
    expect(nx).toBeGreaterThan(0);
    expect(fx).toBeGreaterThan(0);
    expect(nx / 100).toBeGreaterThan(fx / 150);
    expect(magnetShift(from, { x: 300, y: 0 }, 200, 0.5)).toEqual([0, 0]);
    expect(magnetShift(from, from, 200, 0.5)).toEqual([0, 0]);
    // It never pulls past the pointer.
    const [px] = magnetShift(from, { x: 10, y: 0 }, 200, 0.6);
    expect(px).toBeLessThan(10);
  });

  it('Chroma shows full color at the pointer and none beyond its reach', () => {
    expect(chromaAmount(0, 200)).toBe(1);
    expect(chromaAmount(100, 200)).toBeCloseTo(0.5);
    expect(chromaAmount(200, 200)).toBe(0);
    expect(chromaAmount(900, 200)).toBe(0);
    expect(chromaAmount(10, 0)).toBe(0);
  });

  it('Tilt leans toward the pointer and stays small, smaller on bigger cards', () => {
    const small = { left: 0, top: 0, width: 100, height: 100 };
    const big = { left: 0, top: 0, width: 800, height: 400 };
    expect(tiltFor({ x: 50, y: 50 }, small)).toEqual([0, 0]);
    const [rx, ry] = tiltFor({ x: 100, y: 0 }, small);
    expect(ry).toBeGreaterThan(0); // pointer on the right
    expect(rx).toBeGreaterThan(0); // pointer on the top
    expect(Math.abs(ry)).toBeLessThanOrEqual(6);
    expect(Math.abs(tiltFor({ x: 800, y: 0 }, big)[1])).toBeLessThan(Math.abs(ry));
    expect(tiltFor({ x: 5000, y: 5000 }, small).every((v) => Math.abs(v) <= 6)).toBe(true);
  });
});
