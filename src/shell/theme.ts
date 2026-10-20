import { create } from 'zustand';
import { storageGet, storageSet } from '../native/storage';

export const ACCENTS = {
  violet: '#8b5cf6',
  blue: '#3b82f6',
  teal: '#14b8a6',
  rose: '#f43f5e',
  amber: '#f59e0b',
} as const;
export type AccentName = keyof typeof ACCENTS;

export const WALLPAPERS = {
  aurora:
    'radial-gradient(1200px 700px at 15% 10%, #3b2a7a 0%, transparent 60%), radial-gradient(1000px 700px at 90% 90%, #0e5f6b 0%, transparent 55%), #0b0d1a',
  dusk: 'linear-gradient(160deg, #1b1035 0%, #5b2a62 55%, #c2527a 100%)',
  slate: 'linear-gradient(160deg, #0f1115 0%, #1b2030 100%)',
} as const;
export type WallpaperName = keyof typeof WALLPAPERS;

interface ThemeState {
  accent: AccentName;
  wallpaper: WallpaperName;
  loaded: boolean;
  load: () => Promise<void>;
  setAccent: (a: AccentName) => void;
  setWallpaper: (w: WallpaperName) => void;
}

const isAccent = (v: unknown): v is AccentName => typeof v === 'string' && v in ACCENTS;
const isWallpaper = (v: unknown): v is WallpaperName => typeof v === 'string' && v in WALLPAPERS;

export const useTheme = create<ThemeState>((set, get) => ({
  accent: 'violet',
  wallpaper: 'aurora',
  loaded: false,

  load: async () => {
    try {
      const raw = await storageGet('theme');
      const parsed: unknown = raw ? JSON.parse(raw) : null;
      if (parsed && typeof parsed === 'object') {
        const p = parsed as Record<string, unknown>;
        set({
          accent: isAccent(p.accent) ? p.accent : 'violet',
          wallpaper: isWallpaper(p.wallpaper) ? p.wallpaper : 'aurora',
        });
      }
    } catch {
      /* corrupt or unavailable: keep defaults */
    }
    set({ loaded: true });
  },

  setAccent: (accent) => {
    set({ accent });
    void persist(get());
  },
  setWallpaper: (wallpaper) => {
    set({ wallpaper });
    void persist(get());
  },
}));

function persist(s: ThemeState): Promise<void> {
  return storageSet('theme', JSON.stringify({ accent: s.accent, wallpaper: s.wallpaper }));
}
