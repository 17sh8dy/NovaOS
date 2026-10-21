import { create } from 'zustand';
import type { SettingsData } from '../../settings/schema';

/** Rapid changes to the same control (dragging a slider) count as one undo step. */
export const COALESCE_MS = 900;
export const MAX_HISTORY = 100;

export interface HistoryState {
  undo: SettingsData[];
  redo: SettingsData[];
  lastKey: string | null;
  lastAt: number;
}

export const emptyHistory = (): HistoryState => ({ undo: [], redo: [], lastKey: null, lastAt: 0 });

/** Remember `before` as an undo step, unless this is a continuation of the same gesture. */
export function record(
  h: HistoryState,
  before: SettingsData,
  key: string,
  now: number,
): HistoryState {
  const continuing = h.lastKey === key && now - h.lastAt < COALESCE_MS && h.undo.length > 0;
  if (continuing) return { ...h, lastAt: now, redo: [] };
  return {
    undo: [...h.undo, before].slice(-MAX_HISTORY),
    redo: [],
    lastKey: key,
    lastAt: now,
  };
}

export function stepUndo(
  h: HistoryState,
  current: SettingsData,
): { state: HistoryState; data: SettingsData } | null {
  const prev = h.undo[h.undo.length - 1];
  if (!prev) return null;
  return {
    data: prev,
    state: {
      undo: h.undo.slice(0, -1),
      redo: [...h.redo, current].slice(-MAX_HISTORY),
      lastKey: null,
      lastAt: 0,
    },
  };
}

export function stepRedo(
  h: HistoryState,
  current: SettingsData,
): { state: HistoryState; data: SettingsData } | null {
  const next = h.redo[h.redo.length - 1];
  if (!next) return null;
  return {
    data: next,
    state: {
      undo: [...h.undo, current].slice(-MAX_HISTORY),
      redo: h.redo.slice(0, -1),
      lastKey: null,
      lastAt: 0,
    },
  };
}

interface Store extends HistoryState {
  set: (h: HistoryState) => void;
  clear: () => void;
}

/** Undo/redo for the current Studio session. Not saved: closing Studio starts fresh. */
export const useStudioHistory = create<Store>((set) => ({
  ...emptyHistory(),
  set: (h) => set(h),
  clear: () => set(emptyHistory()),
}));
