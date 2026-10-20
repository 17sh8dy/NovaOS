/**
 * The single source of truth for NovaOS settings: shape, defaults, limits, and `sanitize`, which
 * turns ANY value (corrupt file, old version, hand-edited import) into a valid SettingsData.
 */

export const ACCENTS = {
  violet: '#8b5cf6',
  blue: '#3b82f6',
  teal: '#14b8a6',
  green: '#22c55e',
  amber: '#f59e0b',
  rose: '#f43f5e',
} as const;
export type AccentName = keyof typeof ACCENTS;

export const WALLPAPERS = {
  aurora:
    'radial-gradient(1200px 700px at 15% 10%, #3b2a7a 0%, transparent 60%), radial-gradient(1000px 700px at 90% 90%, #0e5f6b 0%, transparent 55%), #0b0d1a',
  dusk: 'linear-gradient(160deg, #1b1035 0%, #5b2a62 55%, #c2527a 100%)',
  ocean: 'linear-gradient(160deg, #04121f 0%, #0a3a5c 60%, #1397a8 100%)',
  forest: 'linear-gradient(160deg, #07130d 0%, #123524 60%, #2f6b4a 100%)',
  slate: 'linear-gradient(160deg, #0f1115 0%, #1b2030 100%)',
  midnight: '#05060d',
} as const;
export type WallpaperName = keyof typeof WALLPAPERS;

export const TASKBAR_SIZES = { compact: 40, default: 48, large: 58 } as const;
export type TaskbarSize = keyof typeof TASKBAR_SIZES;
export type TaskbarPosition = 'bottom' | 'top';
export type IconSize = 'small' | 'medium' | 'large';
export type Corners = 'sharp' | 'rounded' | 'round';
export type ClockFormat = '12h' | '24h';

export const CORNER_PX: Record<Corners, number> = { sharp: 4, rounded: 12, round: 20 };
export const ICON_PX: Record<IconSize, number> = { small: 22, medium: 28, large: 38 };

export const LIMITS = {
  windowOpacity: { min: 0.5, max: 1, step: 0.05 },
  uiScale: { min: 0.85, max: 1.4, step: 0.05 },
  blur: { min: 0, max: 40, step: 2 },
} as const;

export interface SettingsData {
  general: {
    /** Open NovaOS in Immersive Mode. You can always leave with Ctrl+Alt+Q. */
    startInImmersive: boolean;
    clockFormat: ClockFormat;
    showSeconds: boolean;
    showDate: boolean;
  };
  appearance: {
    /** A named accent, or a custom `#rrggbb`. */
    accent: AccentName | string;
    wallpaper: WallpaperName;
    windowOpacity: number;
    blur: number;
    corners: Corners;
  };
  desktop: {
    taskbarPosition: TaskbarPosition;
    taskbarSize: TaskbarSize;
    showDesktopIcons: boolean;
    iconSize: IconSize;
    /** App ids removed from the taskbar and desktop. */
    unpinned: string[];
  };
  files: {
    confirmDelete: boolean;
    showSizes: boolean;
  };
  accessibility: {
    uiScale: number;
    reduceMotion: boolean;
    highContrast: boolean;
  };
}

export const DEFAULTS: SettingsData = {
  general: { startInImmersive: false, clockFormat: '12h', showSeconds: false, showDate: true },
  appearance: {
    accent: 'violet',
    wallpaper: 'aurora',
    windowOpacity: 0.8,
    blur: 24,
    corners: 'rounded',
  },
  desktop: {
    taskbarPosition: 'bottom',
    taskbarSize: 'default',
    showDesktopIcons: true,
    iconSize: 'medium',
    unpinned: [],
  },
  files: { confirmDelete: false, showSizes: true },
  accessibility: { uiScale: 1, reduceMotion: false, highContrast: false },
};

export const SCHEMA_VERSION = 1;

// ---------------------------------------------------------------- sanitizing

const isObj = (v: unknown): v is Record<string, unknown> =>
  typeof v === 'object' && v !== null && !Array.isArray(v);

const bool = (v: unknown, d: boolean): boolean => (typeof v === 'boolean' ? v : d);

function oneOf<T extends string>(v: unknown, allowed: readonly T[], d: T): T {
  return typeof v === 'string' && (allowed as readonly string[]).includes(v) ? (v as T) : d;
}

function num(v: unknown, d: number, min: number, max: number, step: number): number {
  if (typeof v !== 'number' || !Number.isFinite(v)) return d;
  const stepped = Math.round(v / step) * step;
  return Number(Math.min(max, Math.max(min, stepped)).toFixed(4));
}

export const isHexColor = (v: unknown): v is string =>
  typeof v === 'string' && /^#[0-9a-fA-F]{6}$/.test(v);

export function accentColor(accent: string): string {
  return accent in ACCENTS
    ? ACCENTS[accent as AccentName]
    : isHexColor(accent)
      ? accent
      : ACCENTS.violet;
}

/** Valid app ids only; bounded so a hostile import can't bloat storage. */
function idList(v: unknown): string[] {
  if (!Array.isArray(v)) return [];
  const out = new Set<string>();
  for (const x of v) {
    if (typeof x === 'string' && /^[a-z0-9-]{1,40}$/.test(x)) out.add(x);
    if (out.size >= 100) break;
  }
  return [...out];
}

export function sanitize(input: unknown): SettingsData {
  const root = isObj(input) ? input : {};
  const g = isObj(root.general) ? root.general : {};
  const a = isObj(root.appearance) ? root.appearance : {};
  const d = isObj(root.desktop) ? root.desktop : {};
  const f = isObj(root.files) ? root.files : {};
  const x = isObj(root.accessibility) ? root.accessibility : {};
  const D = DEFAULTS;
  return {
    general: {
      startInImmersive: bool(g.startInImmersive, D.general.startInImmersive),
      clockFormat: oneOf(g.clockFormat, ['12h', '24h'], D.general.clockFormat),
      showSeconds: bool(g.showSeconds, D.general.showSeconds),
      showDate: bool(g.showDate, D.general.showDate),
    },
    appearance: {
      accent:
        typeof a.accent === 'string' && (a.accent in ACCENTS || isHexColor(a.accent))
          ? a.accent
          : D.appearance.accent,
      wallpaper: oneOf(
        a.wallpaper,
        Object.keys(WALLPAPERS) as WallpaperName[],
        D.appearance.wallpaper,
      ),
      windowOpacity: num(
        a.windowOpacity,
        D.appearance.windowOpacity,
        LIMITS.windowOpacity.min,
        LIMITS.windowOpacity.max,
        LIMITS.windowOpacity.step,
      ),
      blur: num(a.blur, D.appearance.blur, LIMITS.blur.min, LIMITS.blur.max, LIMITS.blur.step),
      corners: oneOf(a.corners, ['sharp', 'rounded', 'round'], D.appearance.corners),
    },
    desktop: {
      taskbarPosition: oneOf(d.taskbarPosition, ['bottom', 'top'], D.desktop.taskbarPosition),
      taskbarSize: oneOf(
        d.taskbarSize,
        Object.keys(TASKBAR_SIZES) as TaskbarSize[],
        D.desktop.taskbarSize,
      ),
      showDesktopIcons: bool(d.showDesktopIcons, D.desktop.showDesktopIcons),
      iconSize: oneOf(d.iconSize, ['small', 'medium', 'large'], D.desktop.iconSize),
      unpinned: idList(d.unpinned),
    },
    files: {
      confirmDelete: bool(f.confirmDelete, D.files.confirmDelete),
      showSizes: bool(f.showSizes, D.files.showSizes),
    },
    accessibility: {
      uiScale: num(
        x.uiScale,
        D.accessibility.uiScale,
        LIMITS.uiScale.min,
        LIMITS.uiScale.max,
        LIMITS.uiScale.step,
      ),
      reduceMotion: bool(x.reduceMotion, D.accessibility.reduceMotion),
      highContrast: bool(x.highContrast, D.accessibility.highContrast),
    },
  };
}

// ---------------------------------------------------------------- export / import

/** Portable JSON for backup or moving to another machine. */
export function exportSettings(data: SettingsData): string {
  return JSON.stringify({ app: 'novaos', version: SCHEMA_VERSION, settings: data }, null, 2);
}

/** Parse an exported file. Unknown or invalid values fall back to defaults rather than failing. */
export function importSettings(text: string): SettingsData {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    throw new Error('That file is not valid JSON.');
  }
  if (!isObj(parsed) || parsed.app !== 'novaos' || !isObj(parsed.settings)) {
    throw new Error('That file is not a NovaOS settings export.');
  }
  if (typeof parsed.version === 'number' && parsed.version > SCHEMA_VERSION) {
    throw new Error('That export is from a newer version of NovaOS.');
  }
  return sanitize(parsed.settings);
}

/** Immutable deep set for one section field. */
export function patchSection<S extends keyof SettingsData>(
  data: SettingsData,
  section: S,
  patch: Partial<SettingsData[S]>,
): SettingsData {
  return sanitize({ ...data, [section]: { ...data[section], ...patch } });
}

export const taskbarHeight = (size: TaskbarSize): number => TASKBAR_SIZES[size];
