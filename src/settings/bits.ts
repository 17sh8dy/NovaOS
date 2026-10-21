/**
 * "React Bits" effects: small, optional visual touches inspired by the React Bits collection,
 * written for NovaOS (no external code or packages). Every effect is OFF by default, validated
 * here like all other settings, and switched off automatically by Reduce motion / High contrast.
 *
 * This file is plain data: the schema, limits, sanitizing and the catalog the Studio page
 * renders from. The effects themselves live in `src/shell/bits/`.
 */

export type BitColor = 'accent' | string;
export type TextTarget = 'icons' | 'titles' | 'clock';
export type ListStyle = 'slide' | 'fade' | 'pop';

export const TEXT_TARGETS: readonly TextTarget[] = ['icons', 'titles', 'clock'];
export const TEXT_TARGET_LABELS: Record<TextTarget, string> = {
  icons: 'Desktop icon names',
  titles: 'Window titles',
  clock: 'Taskbar clock',
};
export const LIST_STYLES: readonly ListStyle[] = ['slide', 'fade', 'pop'];

interface Range {
  readonly min: number;
  readonly max: number;
  readonly step: number;
}

export const BIT_LIMITS = {
  sparkCount: { min: 4, max: 16, step: 1 },
  sparkLength: { min: 6, max: 40, step: 1 },
  sparkDuration: { min: 200, max: 1200, step: 50 },
  glowSize: { min: 80, max: 500, step: 20 },
  glowIntensity: { min: 0.1, max: 0.8, step: 0.05 },
  swarmCount: { min: 4, max: 40, step: 1 },
  swarmSize: { min: 2, max: 10, step: 1 },
  meshSpacing: { min: 28, max: 90, step: 2 },
  meshStrength: { min: 0.2, max: 1.5, step: 0.1 },
  meshOpacity: { min: 0.05, max: 0.6, step: 0.05 },
  magnetStrength: { min: 0.1, max: 0.6, step: 0.05 },
  magnetRadius: { min: 60, max: 240, step: 10 },
  dockMagnify: { min: 1.1, max: 1.9, step: 0.05 },
  dockRadius: { min: 60, max: 240, step: 10 },
  chromaRadius: { min: 80, max: 400, step: 10 },
  chromaGray: { min: 0.3, max: 1, step: 0.05 },
  bentoRadius: { min: 80, max: 400, step: 10 },
  glowRingRadius: { min: 80, max: 500, step: 10 },
  shinyPeriod: { min: 1, max: 8, step: 0.5 },
  echoSpread: { min: 1, max: 6, step: 1 },
  foldSpeed: { min: 0.5, max: 2.5, step: 0.1 },
  listStagger: { min: 20, max: 120, step: 10 },
} as const satisfies Record<string, Range>;

const L = BIT_LIMITS;

export interface BitsData {
  clickSpark: {
    enabled: boolean;
    color: BitColor;
    count: number;
    length: number;
    duration: number;
  };
  glowCursor: { enabled: boolean; color: BitColor; size: number; intensity: number };
  swarmCursor: { enabled: boolean; color: BitColor; count: number; size: number };
  elasticMesh: {
    enabled: boolean;
    color: BitColor;
    spacing: number;
    strength: number;
    opacity: number;
  };
  magnet: { enabled: boolean; strength: number; radius: number };
  dock: { enabled: boolean; magnify: number; radius: number };
  chromaGrid: { enabled: boolean; radius: number; gray: number };
  magicBento: { enabled: boolean; color: BitColor; radius: number; tilt: boolean };
  borderGlow: { enabled: boolean; color: BitColor; radius: number };
  shinyText: { enabled: boolean; period: number; targets: TextTarget[] };
  echoText: { enabled: boolean; color: BitColor; spread: number; targets: TextTarget[] };
  foldText: { enabled: boolean; speed: number; targets: TextTarget[] };
  animatedList: { enabled: boolean; style: ListStyle; stagger: number };
}

export type BitId = keyof BitsData;

export const BITS_DEFAULTS: BitsData = {
  clickSpark: { enabled: false, color: 'accent', count: 8, length: 14, duration: 500 },
  glowCursor: { enabled: false, color: 'accent', size: 240, intensity: 0.35 },
  swarmCursor: { enabled: false, color: 'accent', count: 14, size: 4 },
  elasticMesh: { enabled: false, color: 'accent', spacing: 48, strength: 0.8, opacity: 0.25 },
  magnet: { enabled: false, strength: 0.3, radius: 120 },
  dock: { enabled: false, magnify: 1.4, radius: 120 },
  chromaGrid: { enabled: false, radius: 200, gray: 1 },
  magicBento: { enabled: false, color: 'accent', radius: 220, tilt: true },
  borderGlow: { enabled: false, color: 'accent', radius: 260 },
  shinyText: { enabled: false, period: 3, targets: ['icons', 'titles'] },
  echoText: { enabled: false, color: 'accent', spread: 2, targets: ['titles'] },
  foldText: { enabled: false, speed: 1, targets: ['icons'] },
  animatedList: { enabled: false, style: 'slide', stagger: 50 },
};

// ---------------------------------------------------------------- sanitizing

const isObj = (v: unknown): v is Record<string, unknown> =>
  typeof v === 'object' && v !== null && !Array.isArray(v);

const HEX = /^#[0-9a-fA-F]{6}$/;

const bool = (v: unknown, d: boolean): boolean => (typeof v === 'boolean' ? v : d);

function num(v: unknown, d: number, r: Range): number {
  if (typeof v !== 'number' || !Number.isFinite(v)) return d;
  const clamped = Math.min(r.max, Math.max(r.min, v));
  // Snap to the step grid so stored values are always ones the slider can show.
  const snapped = r.min + Math.round((clamped - r.min) / r.step) * r.step;
  return Number(Math.min(r.max, snapped).toFixed(4));
}

const color = (v: unknown, d: BitColor): BitColor =>
  v === 'accent' || (typeof v === 'string' && HEX.test(v)) ? (v as string).toLowerCase() : d;

function pick<T extends string>(v: unknown, allowed: readonly T[], d: T): T {
  return typeof v === 'string' && (allowed as readonly string[]).includes(v) ? (v as T) : d;
}

function targets(v: unknown, d: TextTarget[]): TextTarget[] {
  if (!Array.isArray(v)) return [...d];
  const out: TextTarget[] = [];
  for (const t of v) {
    if (typeof t === 'string' && (TEXT_TARGETS as readonly string[]).includes(t)) {
      if (!out.includes(t as TextTarget)) out.push(t as TextTarget);
    }
  }
  return out;
}

/** Make any value into valid React Bits settings: unknown keys dropped, bad values defaulted. */
export function sanitizeBits(input: unknown): BitsData {
  const b = isObj(input) ? input : {};
  const D = BITS_DEFAULTS;
  const sec = (k: BitId): Record<string, unknown> => (isObj(b[k]) ? b[k] : {});
  const cs = sec('clickSpark');
  const gc = sec('glowCursor');
  const sw = sec('swarmCursor');
  const em = sec('elasticMesh');
  const mg = sec('magnet');
  const dk = sec('dock');
  const ch = sec('chromaGrid');
  const mb = sec('magicBento');
  const bg = sec('borderGlow');
  const sh = sec('shinyText');
  const ec = sec('echoText');
  const fo = sec('foldText');
  const al = sec('animatedList');
  return {
    clickSpark: {
      enabled: bool(cs.enabled, D.clickSpark.enabled),
      color: color(cs.color, D.clickSpark.color),
      count: num(cs.count, D.clickSpark.count, L.sparkCount),
      length: num(cs.length, D.clickSpark.length, L.sparkLength),
      duration: num(cs.duration, D.clickSpark.duration, L.sparkDuration),
    },
    glowCursor: {
      enabled: bool(gc.enabled, D.glowCursor.enabled),
      color: color(gc.color, D.glowCursor.color),
      size: num(gc.size, D.glowCursor.size, L.glowSize),
      intensity: num(gc.intensity, D.glowCursor.intensity, L.glowIntensity),
    },
    swarmCursor: {
      enabled: bool(sw.enabled, D.swarmCursor.enabled),
      color: color(sw.color, D.swarmCursor.color),
      count: num(sw.count, D.swarmCursor.count, L.swarmCount),
      size: num(sw.size, D.swarmCursor.size, L.swarmSize),
    },
    elasticMesh: {
      enabled: bool(em.enabled, D.elasticMesh.enabled),
      color: color(em.color, D.elasticMesh.color),
      spacing: num(em.spacing, D.elasticMesh.spacing, L.meshSpacing),
      strength: num(em.strength, D.elasticMesh.strength, L.meshStrength),
      opacity: num(em.opacity, D.elasticMesh.opacity, L.meshOpacity),
    },
    magnet: {
      enabled: bool(mg.enabled, D.magnet.enabled),
      strength: num(mg.strength, D.magnet.strength, L.magnetStrength),
      radius: num(mg.radius, D.magnet.radius, L.magnetRadius),
    },
    dock: {
      enabled: bool(dk.enabled, D.dock.enabled),
      magnify: num(dk.magnify, D.dock.magnify, L.dockMagnify),
      radius: num(dk.radius, D.dock.radius, L.dockRadius),
    },
    chromaGrid: {
      enabled: bool(ch.enabled, D.chromaGrid.enabled),
      radius: num(ch.radius, D.chromaGrid.radius, L.chromaRadius),
      gray: num(ch.gray, D.chromaGrid.gray, L.chromaGray),
    },
    magicBento: {
      enabled: bool(mb.enabled, D.magicBento.enabled),
      color: color(mb.color, D.magicBento.color),
      radius: num(mb.radius, D.magicBento.radius, L.bentoRadius),
      tilt: bool(mb.tilt, D.magicBento.tilt),
    },
    borderGlow: {
      enabled: bool(bg.enabled, D.borderGlow.enabled),
      color: color(bg.color, D.borderGlow.color),
      radius: num(bg.radius, D.borderGlow.radius, L.glowRingRadius),
    },
    shinyText: {
      enabled: bool(sh.enabled, D.shinyText.enabled),
      period: num(sh.period, D.shinyText.period, L.shinyPeriod),
      targets: targets(sh.targets, D.shinyText.targets),
    },
    echoText: {
      enabled: bool(ec.enabled, D.echoText.enabled),
      color: color(ec.color, D.echoText.color),
      spread: num(ec.spread, D.echoText.spread, L.echoSpread),
      targets: targets(ec.targets, D.echoText.targets),
    },
    foldText: {
      enabled: bool(fo.enabled, D.foldText.enabled),
      speed: num(fo.speed, D.foldText.speed, L.foldSpeed),
      targets: targets(fo.targets, D.foldText.targets),
    },
    animatedList: {
      enabled: bool(al.enabled, D.animatedList.enabled),
      style: pick(al.style, LIST_STYLES, D.animatedList.style),
      stagger: num(al.stagger, D.animatedList.stagger, L.listStagger),
    },
  };
}

// ---------------------------------------------------------------- what actually runs

/** Effects that move or follow the pointer. They stop when Reduce motion is on. */
const MOTION_EFFECTS: readonly BitId[] = [
  'clickSpark',
  'glowCursor',
  'swarmCursor',
  'elasticMesh',
  'magnet',
  'dock',
  'chromaGrid',
  'magicBento',
  'borderGlow',
  'shinyText',
  'foldText',
  'animatedList',
];
/** Effects that lower text contrast or color. They stop when High contrast is on. */
const CONTRAST_EFFECTS: readonly BitId[] = ['chromaGrid', 'shinyText', 'echoText', 'foldText'];

/**
 * The settings the effects should really obey. The accessibility settings always win: with
 * Reduce motion nothing moves, and with High contrast nothing washes out text or icons.
 */
export function activeBits(
  bits: BitsData,
  a: { reduceMotion: boolean; highContrast: boolean },
): BitsData {
  if (!a.reduceMotion && !a.highContrast) return bits;
  const out = { ...bits };
  const off = (id: BitId) => {
    out[id] = { ...out[id], enabled: false } as never;
  };
  if (a.reduceMotion) MOTION_EFFECTS.forEach(off);
  if (a.highContrast) CONTRAST_EFFECTS.forEach(off);
  return out;
}

export const anyBitOn = (bits: BitsData): boolean =>
  (Object.keys(bits) as BitId[]).some((k) => bits[k].enabled);

export const countBitsOn = (bits: BitsData): number =>
  (Object.keys(bits) as BitId[]).filter((k) => bits[k].enabled).length;

/** Every effect off, every tweak kept. */
export function allBitsOff(bits: BitsData): BitsData {
  const out = { ...bits };
  for (const k of Object.keys(out) as BitId[]) out[k] = { ...out[k], enabled: false } as never;
  return out;
}

/** The real CSS color for a stored effect color. */
export const bitColor = (c: BitColor): string => (c === 'accent' ? 'var(--accent)' : c);

// ---------------------------------------------------------------- the catalog

export type BitParam =
  | {
      kind: 'slider';
      key: string;
      label: string;
      range: Range;
      unit: string;
      /** Show the value as e.g. `35%` instead of `0.35`. */
      percent?: boolean;
      decimals?: number;
    }
  | { kind: 'color'; key: string; label: string }
  | { kind: 'bool'; key: string; label: string }
  | { kind: 'targets'; key: string; label: string }
  | {
      kind: 'choice';
      key: string;
      label: string;
      options: { value: string; label: string }[];
    };

export type BitCategory = 'Cursor effects' | 'Surfaces & cards' | 'Text effects' | 'Lists';

export interface BitDef {
  id: BitId;
  title: string;
  blurb: string;
  category: BitCategory;
  keywords: string;
  params: BitParam[];
}

export const BITS_CATALOG: BitDef[] = [
  {
    id: 'clickSpark',
    title: 'Click Spark',
    blurb: 'A burst of little sparks wherever you click.',
    category: 'Cursor effects',
    keywords: 'click spark burst particles effect cursor',
    params: [
      { kind: 'color', key: 'color', label: 'Spark color' },
      { kind: 'slider', key: 'count', label: 'Sparks', range: L.sparkCount, unit: '' },
      { kind: 'slider', key: 'length', label: 'Spark length', range: L.sparkLength, unit: 'px' },
      { kind: 'slider', key: 'duration', label: 'Lasts', range: L.sparkDuration, unit: 'ms' },
    ],
  },
  {
    id: 'glowCursor',
    title: 'Glow Cursor',
    blurb: 'A soft light that follows your pointer.',
    category: 'Cursor effects',
    keywords: 'glow cursor light follow spotlight aura pointer',
    params: [
      { kind: 'color', key: 'color', label: 'Glow color' },
      { kind: 'slider', key: 'size', label: 'Size', range: L.glowSize, unit: 'px' },
      {
        kind: 'slider',
        key: 'intensity',
        label: 'Brightness',
        range: L.glowIntensity,
        unit: '%',
        percent: true,
      },
    ],
  },
  {
    id: 'swarmCursor',
    title: 'Swarm Cursor',
    blurb: 'A little swarm of dots that chases your pointer.',
    category: 'Cursor effects',
    keywords: 'swarm cursor dots particles follow trail bugs fireflies pointer',
    params: [
      { kind: 'color', key: 'color', label: 'Dot color' },
      { kind: 'slider', key: 'count', label: 'Dots', range: L.swarmCount, unit: '' },
      { kind: 'slider', key: 'size', label: 'Dot size', range: L.swarmSize, unit: 'px' },
    ],
  },
  {
    id: 'elasticMesh',
    title: 'Elastic Mesh',
    blurb: 'A stretchy grid behind your windows that bends away from the pointer.',
    category: 'Cursor effects',
    keywords: 'elastic mesh grid lines wallpaper background stretch bend rubber net',
    params: [
      { kind: 'color', key: 'color', label: 'Line color' },
      { kind: 'slider', key: 'spacing', label: 'Spacing', range: L.meshSpacing, unit: 'px' },
      { kind: 'slider', key: 'strength', label: 'Stretch', range: L.meshStrength, unit: '×' },
      {
        kind: 'slider',
        key: 'opacity',
        label: 'Line strength',
        range: L.meshOpacity,
        unit: '%',
        percent: true,
      },
    ],
  },
  {
    id: 'magnet',
    title: 'Magnet',
    blurb: 'Desktop icons and taskbar buttons lean toward your pointer.',
    category: 'Surfaces & cards',
    keywords: 'magnet magnetic attract pull icons buttons taskbar lean',
    params: [
      {
        kind: 'slider',
        key: 'strength',
        label: 'Pull',
        range: L.magnetStrength,
        unit: '%',
        percent: true,
      },
      { kind: 'slider', key: 'radius', label: 'Reach', range: L.magnetRadius, unit: 'px' },
    ],
  },
  {
    id: 'dock',
    title: 'Dock',
    blurb: 'Taskbar and Start menu icons grow as your pointer gets close.',
    category: 'Surfaces & cards',
    keywords: 'dock magnify magnification zoom grow macos taskbar icons',
    params: [
      {
        kind: 'slider',
        key: 'magnify',
        label: 'Growth',
        range: L.dockMagnify,
        unit: '×',
        decimals: 2,
      },
      { kind: 'slider', key: 'radius', label: 'Reach', range: L.dockRadius, unit: 'px' },
    ],
  },
  {
    id: 'chromaGrid',
    title: 'Chroma Grid',
    blurb: 'Icons turn gray and bloom into color near your pointer.',
    category: 'Surfaces & cards',
    keywords: 'chroma grid gray grey color reveal spotlight icons saturation',
    params: [
      { kind: 'slider', key: 'radius', label: 'Color reach', range: L.chromaRadius, unit: 'px' },
      {
        kind: 'slider',
        key: 'gray',
        label: 'How gray',
        range: L.chromaGray,
        unit: '%',
        percent: true,
      },
    ],
  },
  {
    id: 'magicBento',
    title: 'Magic Bento',
    blurb: 'Cards in Settings, Studio and the Start menu light up and tilt toward your pointer.',
    category: 'Surfaces & cards',
    keywords: 'magic bento cards spotlight tilt glow panels groups',
    params: [
      { kind: 'color', key: 'color', label: 'Light color' },
      { kind: 'slider', key: 'radius', label: 'Light size', range: L.bentoRadius, unit: 'px' },
      { kind: 'bool', key: 'tilt', label: 'Tilt toward the pointer' },
    ],
  },
  {
    id: 'borderGlow',
    title: 'Border Glow',
    blurb: 'Window edges glow where your pointer is closest.',
    category: 'Surfaces & cards',
    keywords: 'border glow edge outline window frame light pointer',
    params: [
      { kind: 'color', key: 'color', label: 'Glow color' },
      { kind: 'slider', key: 'radius', label: 'Reach', range: L.glowRingRadius, unit: 'px' },
    ],
  },
  {
    id: 'shinyText',
    title: 'Shiny Text',
    blurb: 'A shimmer that sweeps across text.',
    category: 'Text effects',
    keywords: 'shiny text shimmer shine sweep gloss glint',
    params: [
      { kind: 'slider', key: 'period', label: 'One sweep every', range: L.shinyPeriod, unit: 's' },
      { kind: 'targets', key: 'targets', label: 'Where' },
    ],
  },
  {
    id: 'echoText',
    title: 'Echo Text',
    blurb: 'Fading copies trail behind the text, like an echo.',
    category: 'Text effects',
    keywords: 'echo text shadow trail copies repeat ghost',
    params: [
      { kind: 'color', key: 'color', label: 'Echo color' },
      { kind: 'slider', key: 'spread', label: 'Distance', range: L.echoSpread, unit: 'px' },
      { kind: 'targets', key: 'targets', label: 'Where' },
    ],
  },
  {
    id: 'foldText',
    title: 'Fold Text',
    blurb: 'Letters fold over in a wave when you hover them.',
    category: 'Text effects',
    keywords: 'fold text letters flip wave hover rotate 3d',
    params: [
      { kind: 'slider', key: 'speed', label: 'Speed', range: L.foldSpeed, unit: '×' },
      { kind: 'targets', key: 'targets', label: 'Where' },
    ],
  },
  {
    id: 'animatedList',
    title: 'Animated List',
    blurb: 'Items in the Start menu and Nova Files glide in one after another.',
    category: 'Lists',
    keywords: 'animated list items stagger slide fade pop start menu files',
    params: [
      {
        kind: 'choice',
        key: 'style',
        label: 'Style',
        options: [
          { value: 'slide', label: 'Slide' },
          { value: 'fade', label: 'Fade' },
          { value: 'pop', label: 'Pop' },
        ],
      },
      {
        kind: 'slider',
        key: 'stagger',
        label: 'Delay between items',
        range: L.listStagger,
        unit: 'ms',
      },
    ],
  },
];

export const bitDef = (id: BitId): BitDef => BITS_CATALOG.find((b) => b.id === id) as BitDef;
