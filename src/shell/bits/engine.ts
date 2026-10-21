/**
 * The math behind the React Bits effects. Pure functions and small classes, no DOM, so they can
 * be tested without a browser. The components in this folder draw what these compute.
 */

export interface Pt {
  x: number;
  y: number;
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

// ------------------------------------------------------------------ Click Spark

export interface Spark {
  x: number;
  y: number;
  angle: number;
  born: number;
}

/** `count` sparks leaving a click in evenly spaced directions, each nudged a little off-line. */
export function makeSparks(
  x: number,
  y: number,
  count: number,
  now: number,
  rng: () => number = Math.random,
): Spark[] {
  const sparks: Spark[] = [];
  for (let i = 0; i < count; i++) {
    const jitter = (rng() - 0.5) * (Math.PI / count) * 0.8;
    sparks.push({ x, y, angle: (i / count) * Math.PI * 2 + jitter, born: now });
  }
  return sparks;
}

/** Where to draw a spark right now, or null once it has faded out. */
export function sparkLine(
  s: Spark,
  now: number,
  duration: number,
  length: number,
): { x1: number; y1: number; x2: number; y2: number; alpha: number } | null {
  const t = (now - s.born) / duration;
  if (t < 0 || t >= 1) return null;
  const eased = 1 - (1 - t) * (1 - t); // fast start, soft landing
  const reach = length * 2.4;
  const head = eased * reach;
  const tail = Math.max(0, head - length * (1 - t));
  const dx = Math.cos(s.angle);
  const dy = Math.sin(s.angle);
  return {
    x1: s.x + dx * tail,
    y1: s.y + dy * tail,
    x2: s.x + dx * head,
    y2: s.y + dy * head,
    alpha: 1 - t,
  };
}

// ------------------------------------------------------------------ Swarm Cursor

export interface Dot {
  x: number;
  y: number;
  /** How closely it follows (bigger = tighter). */
  lag: number;
  radius: number;
  phase: number;
  speed: number;
}

export function makeSwarm(count: number, at: Pt, rng: () => number = Math.random): Dot[] {
  return Array.from({ length: count }, () => ({
    x: at.x,
    y: at.y,
    lag: 2.5 + rng() * 5,
    radius: 10 + rng() * 26,
    phase: rng() * Math.PI * 2,
    speed: 1.2 + rng() * 2.2,
  }));
}

/** Move every dot toward its own spot circling the pointer. `time` is in seconds. */
export function stepSwarm(dots: Dot[], target: Pt, dt: number, time: number): void {
  const step = clamp(dt, 0, 0.05);
  for (const d of dots) {
    const a = d.phase + time * d.speed;
    const tx = target.x + Math.cos(a) * d.radius;
    const ty = target.y + Math.sin(a * 1.3) * d.radius;
    const k = 1 - Math.exp(-d.lag * step * 4);
    d.x += (tx - d.x) * k;
    d.y += (ty - d.y) * k;
  }
}

// ------------------------------------------------------------------ Elastic Mesh

/** A grid of points joined by springs that bend away from the pointer and snap back. */
export class Mesh {
  cols = 0;
  rows = 0;
  spacing = 0;
  /** Rest position of every point. */
  rx: Float32Array = new Float32Array(0);
  ry: Float32Array = new Float32Array(0);
  /** Current displacement and velocity. */
  dx: Float32Array = new Float32Array(0);
  dy: Float32Array = new Float32Array(0);
  vx: Float32Array = new Float32Array(0);
  vy: Float32Array = new Float32Array(0);

  /** Hard cap so a huge screen with a tiny spacing can't ask for millions of points. */
  static readonly MAX_POINTS = 9000;

  constructor(width: number, height: number, spacing: number) {
    this.resize(width, height, spacing);
  }

  resize(width: number, height: number, spacing: number): void {
    let s = Math.max(8, spacing);
    while ((Math.ceil(width / s) + 1) * (Math.ceil(height / s) + 1) > Mesh.MAX_POINTS) s += 4;
    this.spacing = s;
    this.cols = Math.ceil(width / s) + 1;
    this.rows = Math.ceil(height / s) + 1;
    const n = this.cols * this.rows;
    this.rx = new Float32Array(n);
    this.ry = new Float32Array(n);
    this.dx = new Float32Array(n);
    this.dy = new Float32Array(n);
    this.vx = new Float32Array(n);
    this.vy = new Float32Array(n);
    for (let r = 0; r < this.rows; r++) {
      for (let c = 0; c < this.cols; c++) {
        this.rx[r * this.cols + c] = c * s;
        this.ry[r * this.cols + c] = r * s;
      }
    }
  }

  /**
   * Advance by `dt` seconds with the pointer at `pointer` (or null when it isn't over the
   * screen). Returns how much the mesh is still moving: 0 means it has come to rest.
   */
  step(pointer: Pt | null, strength: number, dt: number): number {
    const h = clamp(dt, 0, 0.033);
    const reach = this.spacing * 3.2;
    const spring = 90;
    const damping = 9;
    let energy = 0;
    for (let i = 0; i < this.rx.length; i++) {
      let ax = -spring * this.dx[i] - damping * this.vx[i];
      let ay = -spring * this.dy[i] - damping * this.vy[i];
      if (pointer) {
        const px = this.rx[i] + this.dx[i] - pointer.x;
        const py = this.ry[i] + this.dy[i] - pointer.y;
        const dist = Math.hypot(px, py);
        if (dist < reach && dist > 0.001) {
          const f = (1 - dist / reach) ** 2 * strength * 5200;
          ax += (px / dist) * f;
          ay += (py / dist) * f;
        }
      }
      this.vx[i] += ax * h;
      this.vy[i] += ay * h;
      this.dx[i] += this.vx[i] * h;
      this.dy[i] += this.vy[i] * h;
      // A point can never be pushed further than a spacing or so; the mesh stays a mesh.
      const m = Math.hypot(this.dx[i], this.dy[i]);
      const cap = this.spacing * 0.9;
      if (m > cap) {
        this.dx[i] *= cap / m;
        this.dy[i] *= cap / m;
      }
      energy += Math.abs(this.vx[i]) + Math.abs(this.vy[i]);
    }
    return energy < this.rx.length * 0.02 ? 0 : energy; // barely moving anywhere: at rest
  }
}

// ------------------------------------------------------------------ Dock / Magnet / Chroma / Bento

/** Distance from a point to a rectangle (0 when inside it). */
export function distToRect(
  p: Pt,
  r: { left: number; top: number; right: number; bottom: number },
): number {
  const dx = Math.max(r.left - p.x, 0, p.x - r.right);
  const dy = Math.max(r.top - p.y, 0, p.y - r.bottom);
  return Math.hypot(dx, dy);
}

/** Smooth 1 → 0 falloff over `radius` (1 at the pointer, 0 at the edge and beyond). */
export function falloff(distance: number, radius: number): number {
  if (radius <= 0 || distance >= radius) return 0;
  return (1 + Math.cos((Math.PI * distance) / radius)) / 2;
}

/** How big a Dock item is: 1 far away, `magnify` right under the pointer. */
export const dockScale = (distance: number, radius: number, magnify: number): number =>
  1 + (magnify - 1) * falloff(distance, radius);

/** How far a Magnet item leans toward the pointer (pixels), or [0, 0] when out of reach. */
export function magnetShift(from: Pt, to: Pt, radius: number, strength: number): [number, number] {
  const dist = Math.hypot(to.x - from.x, to.y - from.y);
  if (dist >= radius || dist < 0.5) return [0, 0];
  const f = (1 - dist / radius) * strength;
  return [(to.x - from.x) * f, (to.y - from.y) * f];
}

/** How much color a Chroma item shows: 1 at the pointer, 0 beyond `radius`. */
export const chromaAmount = (distance: number, radius: number): number =>
  radius <= 0 ? 0 : clamp(1 - distance / radius, 0, 1);

/**
 * Tilt (degrees) toward the pointer for a card. Bigger cards tilt less so they never look broken.
 * Returns [rotateX, rotateY].
 */
export function tiltFor(
  p: Pt,
  r: { left: number; top: number; width: number; height: number },
): [number, number] {
  const max = Math.min(6, 900 / Math.max(r.width, r.height, 1));
  const nx = clamp((p.x - (r.left + r.width / 2)) / (r.width / 2 || 1), -1, 1);
  const ny = clamp((p.y - (r.top + r.height / 2)) / (r.height / 2 || 1), -1, 1);
  return [Number((-ny * max).toFixed(2)), Number((nx * max).toFixed(2))];
}
