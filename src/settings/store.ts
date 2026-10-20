import { create } from 'zustand';
import { storageGet, storageSet } from '../native/storage';
import {
  DEFAULTS,
  exportSettings,
  importSettings,
  patchSection,
  sanitize,
  type SettingsData,
} from './schema';

const KEY = 'settings';
const SAVE_DELAY_MS = 250;

interface SettingsState {
  data: SettingsData;
  loaded: boolean;
  load: () => Promise<void>;
  /** Change one section's fields. Values are validated; invalid ones are ignored. */
  set: <S extends keyof SettingsData>(section: S, patch: Partial<SettingsData[S]>) => void;
  reset: () => void;
  /** Replace everything from exported JSON. Throws a readable Error if it isn't one. */
  importJson: (text: string) => void;
  exportJson: () => string;
}

let saveTimer: ReturnType<typeof setTimeout> | undefined;

function scheduleSave(data: SettingsData): void {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    void storageSet(KEY, JSON.stringify(data)).catch(() => undefined);
  }, SAVE_DELAY_MS);
}

/** Write immediately (used on reset/import so a quick app close can't lose the change). */
function saveNow(data: SettingsData): void {
  clearTimeout(saveTimer);
  void storageSet(KEY, JSON.stringify(data)).catch(() => undefined);
}

export const useSettings = create<SettingsState>((set, get) => ({
  data: DEFAULTS,
  loaded: false,

  load: async () => {
    try {
      const raw = await storageGet(KEY);
      if (raw) {
        set({ data: sanitize(JSON.parse(raw)) });
      } else {
        // First run after the settings upgrade: carry over the old theme-only record.
        const legacy = await storageGet('theme');
        if (legacy) {
          const t = JSON.parse(legacy) as { accent?: unknown; wallpaper?: unknown };
          set({ data: sanitize({ appearance: { accent: t.accent, wallpaper: t.wallpaper } }) });
        }
      }
    } catch {
      /* corrupt or unavailable storage: keep defaults */
    }
    set({ loaded: true });
  },

  set: (section, patch) => {
    const data = patchSection(get().data, section, patch);
    set({ data });
    scheduleSave(data);
  },

  reset: () => {
    set({ data: DEFAULTS });
    saveNow(DEFAULTS);
  },

  importJson: (text) => {
    const data = importSettings(text);
    set({ data });
    saveNow(data);
  },

  exportJson: () => exportSettings(get().data),
}));

/**
 * Props that make an element open something: one click when "Open with a single click" is on,
 * a double click otherwise. Use only for opening programs, files and folders. Settings controls
 * and window chrome never use this.
 */
export function useOpenProps(): (open: () => void) => {
  onClick?: () => void;
  onDoubleClick?: () => void;
} {
  const single = useSettings((s) => s.data.general.singleClickOpen);
  return (open) => (single ? { onClick: open } : { onDoubleClick: open });
}
