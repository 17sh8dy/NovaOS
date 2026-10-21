import { DEFAULTS, pickPreset, sanitize, type PresetData } from './schema';

export interface BuiltinLook {
  id: string;
  name: string;
  description: string;
  data: PresetData;
}

/** Build a look from partial changes on top of the defaults, validated like any other input. */
const look = (changes: Record<string, Record<string, unknown>>): PresetData =>
  pickPreset(
    sanitize(
      Object.fromEntries(
        Object.keys(DEFAULTS).map((k) => [
          k,
          { ...(DEFAULTS as unknown as Record<string, object>)[k], ...(changes[k] ?? {}) },
        ]),
      ),
    ),
  );

/** Ready-made starting points. They never include personal wallpapers. */
export const BUILTIN_LOOKS: BuiltinLook[] = [
  {
    id: 'nova-aurora',
    name: 'Nova Aurora',
    description: 'The default: violet glass over a soft aurora.',
    data: look({}),
  },
  {
    id: 'midnight-glass',
    name: 'Midnight Glass',
    description: 'Near-black, very glassy, cyan accent.',
    data: look({
      appearance: {
        accent: 'teal',
        wallpaper: 'midnight',
        windowOpacity: 0.65,
        blur: 36,
        surfaceColor: '#0a0d18',
      },
      windows: { shadow: 'strong' },
      desktop: { taskbarStyle: 'floating', taskbarOpacity: 0.4 },
    }),
  },
  {
    id: 'ocean-calm',
    name: 'Ocean Calm',
    description: 'Deep blues, round corners, gentle motion.',
    data: look({
      appearance: { accent: 'blue', wallpaper: 'ocean', corners: 'round', surfaceColor: '#0b1b2b' },
      motion: { speed: 0.8, windowAnimation: 'fade' },
    }),
  },
  {
    id: 'forest-focus',
    name: 'Forest Focus',
    description: 'Green and quiet: dimmed wallpaper, no distractions.',
    data: look({
      appearance: {
        accent: 'green',
        wallpaper: 'forest',
        wallpaperDim: 0.2,
        surfaceColor: '#0e1a14',
      },
      desktop: { iconLabels: true, iconSize: 'small' },
      motion: { windowAnimation: 'none' },
    }),
  },
  {
    id: 'neon-dusk',
    name: 'Neon Dusk',
    description: 'Warm sunset with a glowing rose cursor and click ripples.',
    data: look({
      appearance: { accent: 'rose', wallpaper: 'dusk', selectionColor: '#f43f5e' },
      cursor: { style: 'dot', color: '#f43f5e', outline: '#ffffff', ripple: true },
      motion: { speed: 1.4, windowAnimation: 'slide' },
    }),
  },
  {
    id: 'crisp-contrast',
    name: 'Crisp Contrast',
    description: 'Solid windows, strong edges, larger text: easy on the eyes.',
    data: look({
      appearance: {
        accent: 'amber',
        wallpaper: 'midnight',
        windowOpacity: 1,
        blur: 0,
        corners: 'sharp',
      },
      windows: { shadow: 'none', titleBar: 'large', activeBorder: 'accent' },
      accessibility: { highContrast: true, uiScale: 1.15 },
      motion: { windowAnimation: 'none' },
    }),
  },
];
