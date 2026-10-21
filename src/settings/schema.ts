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

export type WallpaperFit = 'fill' | 'fit' | 'stretch' | 'tile' | 'center';

/** Where custom wallpapers are stored, inside the Nova Files root. */
export const WALLPAPER_ROOT = 'nova';
export const WALLPAPER_DIR = 'Pictures/Wallpapers';

/** The only names a stored wallpaper can have; the Rust side normalizes every upload to this. */
const WALLPAPER_FILE_RE = /^[A-Za-z0-9_()-][A-Za-z0-9 ._()-]{0,83}\.(png|jpe?g|webp|gif|bmp)$/i;
export const isWallpaperFileName = (name: unknown): name is string =>
  typeof name === 'string' && WALLPAPER_FILE_RE.test(name);

/** `file:<name>` is a custom wallpaper; returns the file name, or null for presets/invalid. */
export function wallpaperFile(wallpaper: string): string | null {
  if (!wallpaper.startsWith('file:')) return null;
  const name = wallpaper.slice(5);
  return isWallpaperFileName(name) ? name : null;
}

export const TASKBAR_SIZES = { compact: 40, default: 48, large: 58 } as const;
export type TaskbarSize = keyof typeof TASKBAR_SIZES;
export type TaskbarPosition = 'bottom' | 'top';
export type TaskbarStyle = 'docked' | 'floating';
export type IconSize = 'small' | 'medium' | 'large';
export type IconSpacing = 'tight' | 'normal' | 'roomy';
export type Corners = 'sharp' | 'rounded' | 'round';
export type ClockFormat = '12h' | '24h';
export type WindowShadow = 'none' | 'soft' | 'strong';
export type TitleBar = 'compact' | 'default' | 'large';
export type ActiveBorder = 'accent' | 'subtle' | 'off';
export type FontChoice = 'system' | 'sans' | 'serif' | 'mono' | 'rounded';
export type WindowAnimation = 'scale' | 'fade' | 'slide' | 'none';
export type CursorStyle = 'system' | 'classic' | 'dot' | 'crosshair';
export type SoundPack = 'soft' | 'click' | 'chime' | 'retro';
export type StudioSidebar = 'full' | 'compact';
export type PreviewPosition = 'top' | 'side';

export const CORNER_PX: Record<Corners, number> = { sharp: 4, rounded: 12, round: 20 };
export const ICON_PX: Record<IconSize, number> = { small: 22, medium: 28, large: 38 };
export const ICON_GAP_PX: Record<IconSpacing, number> = { tight: 2, normal: 8, roomy: 18 };
export const TITLEBAR_PX: Record<TitleBar, number> = { compact: 30, default: 36, large: 44 };
export const SHADOWS: Record<WindowShadow, string> = {
  none: 'none',
  soft: '0 10px 30px rgb(0 0 0 / 0.35)',
  strong: '0 24px 70px rgb(0 0 0 / 0.65), 0 4px 14px rgb(0 0 0 / 0.5)',
};
/** System font stacks only: nothing is downloaded. */
export const FONTS: Record<FontChoice, string> = {
  system: "'Segoe UI Variable', 'Segoe UI', system-ui, sans-serif",
  sans: "Arial, Helvetica, 'Liberation Sans', sans-serif",
  serif: "Georgia, 'Times New Roman', 'Liberation Serif', serif",
  mono: "'Cascadia Code', Consolas, 'Courier New', monospace",
  rounded: "'Segoe UI Rounded', 'Trebuchet MS', 'Varela Round', system-ui, sans-serif",
};
export const DEFAULT_SURFACE = '#141622';

/** Apps pinned out of the box, in order. Users pin or unpin any app from here. */
export const DEFAULT_PINNED = ['files', 'notes', 'calculator', 'studio', 'settings'] as const;

export const LIMITS = {
  windowOpacity: { min: 0.5, max: 1, step: 0.05 },
  uiScale: { min: 0.85, max: 1.4, step: 0.05 },
  blur: { min: 0, max: 40, step: 2 },
  wallpaperDim: { min: 0, max: 0.6, step: 0.05 },
  selectionOpacity: { min: 0.05, max: 0.5, step: 0.05 },
  taskbarOpacity: { min: 0.2, max: 1, step: 0.05 },
  motionSpeed: { min: 0.5, max: 2.5, step: 0.1 },
  cursorSize: { min: 20, max: 48, step: 4 },
  volume: { min: 0, max: 1, step: 0.05 },
} as const;

export const MAX_PRESETS = 30;
export const MAX_FAVORITES = 40;
export const MAX_PINNED = 40;

/** A saved look: every customization section, none of the personal/system ones. */
export const PRESET_SECTIONS = [
  'appearance',
  'windows',
  'desktop',
  'accessibility',
  'interface',
  'motion',
  'cursor',
  'sounds',
] as const;
export type PresetSection = (typeof PRESET_SECTIONS)[number];
export type PresetData = Pick<SettingsData, PresetSection>;

export interface Preset {
  id: string;
  name: string;
  data: PresetData;
}

export interface SettingsData {
  general: {
    /** Open NovaOS in Immersive Mode. You can always leave with Ctrl+Alt+Q. */
    startInImmersive: boolean;
    clockFormat: ClockFormat;
    showSeconds: boolean;
    showDate: boolean;
    /** Open programs, files and folders with one click instead of two. */
    singleClickOpen: boolean;
  };
  appearance: {
    /** A named accent, or a custom `#rrggbb`. */
    accent: AccentName | string;
    /** A preset name, or `file:<name>` for a custom image. */
    wallpaper: WallpaperName | string;
    wallpaperFit: WallpaperFit;
    /** Darkens the wallpaper so icons stay readable (0 = none). */
    wallpaperDim: number;
    /** The base color of windows, menus and panels. */
    surfaceColor: string;
    windowOpacity: number;
    blur: number;
    corners: Corners;
    /** The drag-to-select box on the desktop. */
    selectionEnabled: boolean;
    /** `accent` follows the accent color; otherwise `#rrggbb`. */
    selectionColor: 'accent' | string;
    selectionOpacity: number;
  };
  windows: {
    shadow: WindowShadow;
    titleBar: TitleBar;
    activeBorder: ActiveBorder;
  };
  desktop: {
    taskbarPosition: TaskbarPosition;
    taskbarSize: TaskbarSize;
    taskbarStyle: TaskbarStyle;
    taskbarOpacity: number;
    /** Show window titles next to taskbar icons. */
    taskbarLabels: boolean;
    showDesktopIcons: boolean;
    iconSize: IconSize;
    iconLabels: boolean;
    iconSpacing: IconSpacing;
    /** App ids on the taskbar, in order. */
    taskbarPinned: string[];
    /** App ids on the desktop, in order. */
    desktopPinned: string[];
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
  interface: {
    font: FontChoice;
  };
  motion: {
    /** 1 = normal; higher is faster. */
    speed: number;
    windowAnimation: WindowAnimation;
  };
  cursor: {
    style: CursorStyle;
    color: string;
    outline: string;
    size: number;
    /** A ripple at every click. */
    ripple: boolean;
    /** `accent` follows the accent color; otherwise `#rrggbb`. */
    rippleColor: 'accent' | string;
  };
  sounds: {
    enabled: boolean;
    volume: number;
    pack: SoundPack;
    /** A soft tick on button clicks (window sounds are separate). */
    clicks: boolean;
  };
  studio: {
    /** On: changes apply as you make them. Off: changes wait for Apply. */
    instantApply: boolean;
    /** Show precise numeric fields and extra controls. */
    advanced: boolean;
    sidebar: StudioSidebar;
    showPreview: boolean;
    previewPosition: PreviewPosition;
    /** Control ids pinned to the Studio home page. */
    favorites: string[];
    presets: Preset[];
  };
}

export const DEFAULTS: SettingsData = {
  general: {
    startInImmersive: false,
    clockFormat: '12h',
    showSeconds: false,
    showDate: true,
    singleClickOpen: false,
  },
  appearance: {
    accent: 'violet',
    wallpaper: 'aurora',
    wallpaperFit: 'fill',
    wallpaperDim: 0,
    surfaceColor: DEFAULT_SURFACE,
    windowOpacity: 0.8,
    blur: 24,
    corners: 'rounded',
    selectionEnabled: true,
    selectionColor: 'accent',
    selectionOpacity: 0.2,
  },
  windows: { shadow: 'soft', titleBar: 'default', activeBorder: 'accent' },
  desktop: {
    taskbarPosition: 'bottom',
    taskbarSize: 'default',
    taskbarStyle: 'docked',
    taskbarOpacity: 0.5,
    taskbarLabels: true,
    showDesktopIcons: true,
    iconSize: 'medium',
    iconLabels: true,
    iconSpacing: 'normal',
    taskbarPinned: [...DEFAULT_PINNED],
    desktopPinned: [...DEFAULT_PINNED],
  },
  files: { confirmDelete: false, showSizes: true },
  accessibility: { uiScale: 1, reduceMotion: false, highContrast: false },
  interface: { font: 'system' },
  motion: { speed: 1, windowAnimation: 'scale' },
  cursor: {
    style: 'system',
    color: '#ffffff',
    outline: '#000000',
    size: 32,
    ripple: false,
    rippleColor: 'accent',
  },
  sounds: { enabled: false, volume: 0.5, pack: 'soft', clicks: false },
  studio: {
    instantApply: true,
    advanced: false,
    sidebar: 'full',
    showPreview: true,
    previewPosition: 'top',
    favorites: [],
    presets: [],
  },
};

export const SCHEMA_VERSION = 2;

// ---------------------------------------------------------------- sanitizing

const isObj = (v: unknown): v is Record<string, unknown> =>
  typeof v === 'object' && v !== null && !Array.isArray(v);

const bool = (v: unknown, d: boolean): boolean => (typeof v === 'boolean' ? v : d);

function oneOf<T extends string>(v: unknown, allowed: readonly T[], d: T): T {
  return typeof v === 'string' && (allowed as readonly string[]).includes(v) ? (v as T) : d;
}

function num(
  v: unknown,
  d: number,
  limits: { readonly min: number; readonly max: number; readonly step: number },
): number {
  if (typeof v !== 'number' || !Number.isFinite(v)) return d;
  const stepped = Math.round(v / limits.step) * limits.step;
  return Number(Math.min(limits.max, Math.max(limits.min, stepped)).toFixed(4));
}

export const isHexColor = (v: unknown): v is string =>
  typeof v === 'string' && /^#[0-9a-fA-F]{6}$/.test(v);

const hex = (v: unknown, d: string): string => (isHexColor(v) ? v : d);

export function accentColor(accent: string): string {
  return accent in ACCENTS
    ? ACCENTS[accent as AccentName]
    : isHexColor(accent)
      ? accent
      : ACCENTS.violet;
}

/** Strings matching `re`, de-duplicated, order kept, at most `max`. */
function idList(v: unknown, re: RegExp, max: number): string[] {
  if (!Array.isArray(v)) return [];
  const out = new Set<string>();
  for (const x of v) {
    if (typeof x === 'string' && re.test(x)) out.add(x);
    if (out.size >= max) break;
  }
  return [...out];
}

const APP_ID_RE = /^[a-z0-9-]{1,40}$/;
const CONTROL_ID_RE = /^[a-z0-9.-]{1,60}$/;
const PRESET_ID_RE = /^[a-z0-9-]{1,40}$/;

/** Names are shown in the UI as text: strip control characters, trim, and bound the length. */
export function cleanName(v: unknown, max = 40): string | null {
  if (typeof v !== 'string') return null;
  const noControls = [...v]
    .filter((ch) => ch.charCodeAt(0) > 31 && ch.charCodeAt(0) !== 127)
    .join('');
  const s = noControls.trim().slice(0, max).trim();
  return s.length > 0 ? s : null;
}

export function pickPreset(data: SettingsData): PresetData {
  const out = {} as Record<string, unknown>;
  for (const k of PRESET_SECTIONS) out[k] = data[k];
  return out as PresetData;
}

function sanitizePresets(v: unknown): Preset[] {
  if (!Array.isArray(v)) return [];
  const seen = new Set<string>();
  const out: Preset[] = [];
  for (const p of v) {
    if (!isObj(p)) continue;
    const name = cleanName(p.name);
    if (!name || typeof p.id !== 'string' || !PRESET_ID_RE.test(p.id) || seen.has(p.id)) continue;
    seen.add(p.id);
    out.push({ id: p.id, name, data: pickPreset(sanitize({ ...(isObj(p.data) ? p.data : {}) })) });
    if (out.length >= MAX_PRESETS) break;
  }
  return out;
}

export function sanitize(input: unknown): SettingsData {
  const root = isObj(input) ? input : {};
  const g = isObj(root.general) ? root.general : {};
  const a = isObj(root.appearance) ? root.appearance : {};
  const w = isObj(root.windows) ? root.windows : {};
  const d = isObj(root.desktop) ? root.desktop : {};
  const f = isObj(root.files) ? root.files : {};
  const x = isObj(root.accessibility) ? root.accessibility : {};
  const i = isObj(root.interface) ? root.interface : {};
  const m = isObj(root.motion) ? root.motion : {};
  const c = isObj(root.cursor) ? root.cursor : {};
  const s = isObj(root.sounds) ? root.sounds : {};
  const st = isObj(root.studio) ? root.studio : {};
  const D = DEFAULTS;

  // Before the pin lists existed there was only an "unpinned" list; honour it once.
  const legacyUnpinned = idList(d.unpinned, APP_ID_RE, 100);
  const migratedPins = DEFAULT_PINNED.filter((id) => !legacyUnpinned.includes(id));
  const pins = (v: unknown): string[] =>
    Array.isArray(v) ? idList(v, APP_ID_RE, MAX_PINNED) : [...migratedPins];

  return {
    general: {
      startInImmersive: bool(g.startInImmersive, D.general.startInImmersive),
      clockFormat: oneOf(g.clockFormat, ['12h', '24h'], D.general.clockFormat),
      showSeconds: bool(g.showSeconds, D.general.showSeconds),
      showDate: bool(g.showDate, D.general.showDate),
      singleClickOpen: bool(g.singleClickOpen, D.general.singleClickOpen),
    },
    appearance: {
      accent:
        typeof a.accent === 'string' && (a.accent in ACCENTS || isHexColor(a.accent))
          ? a.accent
          : D.appearance.accent,
      wallpaper:
        typeof a.wallpaper === 'string' &&
        (a.wallpaper in WALLPAPERS || wallpaperFile(a.wallpaper) !== null)
          ? a.wallpaper
          : D.appearance.wallpaper,
      wallpaperFit: oneOf(
        a.wallpaperFit,
        ['fill', 'fit', 'stretch', 'tile', 'center'],
        D.appearance.wallpaperFit,
      ),
      wallpaperDim: num(a.wallpaperDim, D.appearance.wallpaperDim, LIMITS.wallpaperDim),
      surfaceColor: hex(a.surfaceColor, D.appearance.surfaceColor),
      windowOpacity: num(a.windowOpacity, D.appearance.windowOpacity, LIMITS.windowOpacity),
      blur: num(a.blur, D.appearance.blur, LIMITS.blur),
      corners: oneOf(a.corners, ['sharp', 'rounded', 'round'], D.appearance.corners),
      selectionEnabled: bool(a.selectionEnabled, D.appearance.selectionEnabled),
      selectionColor:
        a.selectionColor === 'accent' || isHexColor(a.selectionColor)
          ? a.selectionColor
          : D.appearance.selectionColor,
      selectionOpacity: num(
        a.selectionOpacity,
        D.appearance.selectionOpacity,
        LIMITS.selectionOpacity,
      ),
    },
    windows: {
      shadow: oneOf(w.shadow, ['none', 'soft', 'strong'], D.windows.shadow),
      titleBar: oneOf(w.titleBar, ['compact', 'default', 'large'], D.windows.titleBar),
      activeBorder: oneOf(w.activeBorder, ['accent', 'subtle', 'off'], D.windows.activeBorder),
    },
    desktop: {
      taskbarPosition: oneOf(d.taskbarPosition, ['bottom', 'top'], D.desktop.taskbarPosition),
      taskbarSize: oneOf(
        d.taskbarSize,
        Object.keys(TASKBAR_SIZES) as TaskbarSize[],
        D.desktop.taskbarSize,
      ),
      taskbarStyle: oneOf(d.taskbarStyle, ['docked', 'floating'], D.desktop.taskbarStyle),
      taskbarOpacity: num(d.taskbarOpacity, D.desktop.taskbarOpacity, LIMITS.taskbarOpacity),
      taskbarLabels: bool(d.taskbarLabels, D.desktop.taskbarLabels),
      showDesktopIcons: bool(d.showDesktopIcons, D.desktop.showDesktopIcons),
      iconSize: oneOf(d.iconSize, ['small', 'medium', 'large'], D.desktop.iconSize),
      iconLabels: bool(d.iconLabels, D.desktop.iconLabels),
      iconSpacing: oneOf(d.iconSpacing, ['tight', 'normal', 'roomy'], D.desktop.iconSpacing),
      taskbarPinned: pins(d.taskbarPinned),
      desktopPinned: pins(d.desktopPinned),
    },
    files: {
      confirmDelete: bool(f.confirmDelete, D.files.confirmDelete),
      showSizes: bool(f.showSizes, D.files.showSizes),
    },
    accessibility: {
      uiScale: num(x.uiScale, D.accessibility.uiScale, LIMITS.uiScale),
      reduceMotion: bool(x.reduceMotion, D.accessibility.reduceMotion),
      highContrast: bool(x.highContrast, D.accessibility.highContrast),
    },
    interface: {
      font: oneOf(i.font, ['system', 'sans', 'serif', 'mono', 'rounded'], D.interface.font),
    },
    motion: {
      speed: num(m.speed, D.motion.speed, LIMITS.motionSpeed),
      windowAnimation: oneOf(
        m.windowAnimation,
        ['scale', 'fade', 'slide', 'none'],
        D.motion.windowAnimation,
      ),
    },
    cursor: {
      style: oneOf(c.style, ['system', 'classic', 'dot', 'crosshair'], D.cursor.style),
      color: hex(c.color, D.cursor.color),
      outline: hex(c.outline, D.cursor.outline),
      size: num(c.size, D.cursor.size, LIMITS.cursorSize),
      ripple: bool(c.ripple, D.cursor.ripple),
      rippleColor:
        c.rippleColor === 'accent' || isHexColor(c.rippleColor)
          ? c.rippleColor
          : D.cursor.rippleColor,
    },
    sounds: {
      enabled: bool(s.enabled, D.sounds.enabled),
      volume: num(s.volume, D.sounds.volume, LIMITS.volume),
      pack: oneOf(s.pack, ['soft', 'click', 'chime', 'retro'], D.sounds.pack),
      clicks: bool(s.clicks, D.sounds.clicks),
    },
    studio: {
      instantApply: bool(st.instantApply, D.studio.instantApply),
      advanced: bool(st.advanced, D.studio.advanced),
      sidebar: oneOf(st.sidebar, ['full', 'compact'], D.studio.sidebar),
      showPreview: bool(st.showPreview, D.studio.showPreview),
      previewPosition: oneOf(st.previewPosition, ['top', 'side'], D.studio.previewPosition),
      favorites: idList(st.favorites, CONTROL_ID_RE, MAX_FAVORITES),
      presets: sanitizePresets(st.presets),
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

/** A shareable look: one preset in its own small file. */
export function exportLook(name: string, data: PresetData): string {
  return JSON.stringify(
    { app: 'novaos', kind: 'look', version: SCHEMA_VERSION, name, look: data },
    null,
    2,
  );
}

export function importLook(text: string): { name: string; data: PresetData } {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    throw new Error('That file is not valid JSON.');
  }
  if (!isObj(parsed) || parsed.app !== 'novaos' || parsed.kind !== 'look' || !isObj(parsed.look)) {
    throw new Error('That file is not a NovaOS look.');
  }
  if (typeof parsed.version === 'number' && parsed.version > SCHEMA_VERSION) {
    throw new Error('That look is from a newer version of NovaOS.');
  }
  return {
    name: cleanName(parsed.name) ?? 'Imported look',
    data: pickPreset(sanitize(parsed.look)),
  };
}

/** Immutable set of one section's fields. */
export function patchSection<S extends keyof SettingsData>(
  data: SettingsData,
  section: S,
  patch: Partial<SettingsData[S]>,
): SettingsData {
  return sanitize({ ...data, [section]: { ...data[section], ...patch } });
}

/** Put a saved look onto the current settings, leaving general, files and studio alone. */
export function applyLook(data: SettingsData, look: PresetData): SettingsData {
  return sanitize({ ...data, ...look });
}

export const taskbarHeight = (size: TaskbarSize): number => TASKBAR_SIZES[size];

/** Space the taskbar takes from the desktop: its height, plus the gap when it floats. */
export function taskbarReserved(
  d: Pick<SettingsData['desktop'], 'taskbarSize' | 'taskbarStyle'>,
): number {
  return taskbarHeight(d.taskbarSize) + (d.taskbarStyle === 'floating' ? 16 : 0);
}

/** The actual CSS color for the selection box. */
export function selectionColor(a: SettingsData['appearance']): string {
  return a.selectionColor === 'accent' ? accentColor(a.accent) : a.selectionColor;
}

export function rippleColor(a: SettingsData['appearance'], c: SettingsData['cursor']): string {
  return c.rippleColor === 'accent' ? accentColor(a.accent) : c.rippleColor;
}

/** "#rrggbb" -> "r g b" for use inside rgb(r g b / alpha). */
export function hexToRgbTriplet(h: string): string {
  const n = parseInt(h.slice(1), 16);
  return `${(n >> 16) & 255} ${(n >> 8) & 255} ${n & 255}`;
}
