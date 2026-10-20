import { create } from 'zustand';

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface NovaWindow {
  id: string;
  appId: string;
  title: string;
  rect: Rect;
  minimized: boolean;
  maximized: boolean;
  /** Launch arguments, e.g. which file an editor should open. */
  args?: Record<string, string>;
  /** Higher = nearer the front. Unique per window. */
  z: number;
}

export interface OpenOptions {
  appId: string;
  title: string;
  size: { width: number; height: number };
  /** Reuse (and focus) an existing window of this app instead of opening another. */
  singleton?: boolean;
  args?: Record<string, string>;
}

export interface Size {
  width: number;
  height: number;
}

export const MIN_WINDOW: Size = { width: 240, height: 160 };
/** Pixels of window that must stay reachable so it can never be lost off-screen. */
export const TITLEBAR_GRAB = 48;

interface WindowState {
  windows: Record<string, NovaWindow>;
  activeId: string | null;
  nextZ: number;
  counter: number;
  open: (opts: OpenOptions, desktop: Size) => string;
  close: (id: string) => void;
  focus: (id: string) => void;
  minimize: (id: string) => void;
  restore: (id: string) => void;
  toggleMaximize: (id: string) => void;
  /** Taskbar click: minimized -> restore, background -> focus, active -> minimize. */
  taskbarActivate: (id: string) => void;
  move: (id: string, x: number, y: number, desktop: Size) => void;
  setRect: (id: string, rect: Rect) => void;
  closeAll: () => void;
}

/** Topmost non-minimized window, or null. */
function topVisible(windows: Record<string, NovaWindow>, excludeId?: string): string | null {
  let best: NovaWindow | null = null;
  for (const w of Object.values(windows)) {
    if (w.id === excludeId || w.minimized) continue;
    if (!best || w.z > best.z) best = w;
  }
  return best ? best.id : null;
}

export const useWindows = create<WindowState>((set, get) => ({
  windows: {},
  activeId: null,
  nextZ: 1,
  counter: 0,

  open: (opts, desktop) => {
    if (opts.singleton) {
      const existing = Object.values(get().windows).find((w) => w.appId === opts.appId);
      if (existing) {
        get().focus(existing.id);
        return existing.id;
      }
    }
    const n = get().counter + 1;
    const id = `w${n}`;
    set((s) => {
      const width = Math.min(opts.size.width, Math.max(MIN_WINDOW.width, desktop.width));
      const height = Math.min(opts.size.height, Math.max(MIN_WINDOW.height, desktop.height));
      // Cascade new windows so they never stack exactly on top of each other.
      const step = ((n - 1) % 8) * 32;
      const x = Math.max(0, Math.min(48 + step, desktop.width - width));
      const y = Math.max(0, Math.min(32 + step, desktop.height - height));
      const win: NovaWindow = {
        id,
        appId: opts.appId,
        title: opts.title,
        rect: { x, y, width, height },
        minimized: false,
        maximized: false,
        args: opts.args,
        z: s.nextZ,
      };
      return {
        windows: { ...s.windows, [id]: win },
        activeId: id,
        nextZ: s.nextZ + 1,
        counter: n,
      };
    });
    return id;
  },

  close: (id) =>
    set((s) => {
      if (!s.windows[id]) return s;
      const rest = { ...s.windows };
      delete rest[id];
      const activeId = s.activeId === id ? topVisible(rest) : s.activeId;
      return { windows: rest, activeId };
    }),

  focus: (id) =>
    set((s) => {
      const w = s.windows[id];
      if (!w) return s;
      if (s.activeId === id && !w.minimized) return s;
      return {
        windows: { ...s.windows, [id]: { ...w, minimized: false, z: s.nextZ } },
        activeId: id,
        nextZ: s.nextZ + 1,
      };
    }),

  minimize: (id) =>
    set((s) => {
      const w = s.windows[id];
      if (!w || w.minimized) return s;
      const windows = { ...s.windows, [id]: { ...w, minimized: true } };
      return { windows, activeId: s.activeId === id ? topVisible(windows, id) : s.activeId };
    }),

  restore: (id) => get().focus(id),

  toggleMaximize: (id) =>
    set((s) => {
      const w = s.windows[id];
      if (!w) return s;
      return { windows: { ...s.windows, [id]: { ...w, maximized: !w.maximized } } };
    }),

  taskbarActivate: (id) => {
    const { windows, activeId } = get();
    const w = windows[id];
    if (!w) return;
    if (!w.minimized && activeId === id) get().minimize(id);
    else get().focus(id);
  },

  move: (id, x, y, desktop) =>
    set((s) => {
      const w = s.windows[id];
      if (!w || w.maximized) return s;
      const cx = Math.max(TITLEBAR_GRAB - w.rect.width, Math.min(x, desktop.width - TITLEBAR_GRAB));
      const cy = Math.max(0, Math.min(y, desktop.height - TITLEBAR_GRAB));
      return { windows: { ...s.windows, [id]: { ...w, rect: { ...w.rect, x: cx, y: cy } } } };
    }),

  setRect: (id, rect) =>
    set((s) => {
      const w = s.windows[id];
      if (!w || w.maximized) return s;
      const clamped = {
        ...rect,
        width: Math.max(MIN_WINDOW.width, rect.width),
        height: Math.max(MIN_WINDOW.height, rect.height),
      };
      return { windows: { ...s.windows, [id]: { ...w, rect: clamped } } };
    }),

  closeAll: () => set({ windows: {}, activeId: null }),
}));
