import { create } from 'zustand';
import { isTauri, native } from '../native/bridge';

/** `shell` is reserved for the future and is refused in this release. */
export type DisplayMode = 'windowed' | 'immersive' | 'shell';

interface DisplayState {
  mode: DisplayMode;
  error: string | null;
  setMode: (mode: DisplayMode) => Promise<void>;
  /** Always safe to call; used by every exit affordance. */
  exitImmersive: () => Promise<void>;
}

async function applyFullscreen(on: boolean): Promise<void> {
  if (isTauri()) {
    await native<void>('set_fullscreen', { fullscreen: on });
  } else if (on) {
    await document.documentElement.requestFullscreen();
  } else if (document.fullscreenElement) {
    await document.exitFullscreen();
  }
}

export const useDisplay = create<DisplayState>((set) => ({
  mode: 'windowed',
  error: null,

  setMode: async (mode) => {
    if (mode === 'shell') {
      set({ error: 'Shell Mode is not available yet.' });
      return;
    }
    try {
      await applyFullscreen(mode === 'immersive');
      set({ mode, error: null });
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String(e) });
    }
  },

  exitImmersive: async () => {
    // Even if the native call fails, flip the UI back so the user is never stranded.
    try {
      await applyFullscreen(false);
    } catch {
      /* ignore: fullscreen may already be off */
    }
    set({ mode: 'windowed', error: null });
  },
}));

/** Ctrl+Alt+Q always leaves Immersive Mode, from anywhere, regardless of focused app. */
export function isExitChord(e: KeyboardEvent): boolean {
  return e.ctrlKey && e.altKey && !e.shiftKey && e.key.toLowerCase() === 'q';
}
