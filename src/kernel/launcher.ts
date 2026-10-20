import type { Size } from './windowStore';
import { useWindows } from './windowStore';
import { getApp } from './appRegistry';

let desktopSize: Size = { width: 1024, height: 640 };

/** The Desktop reports its usable area so any code can open windows that fit. */
export function setDesktopSize(size: Size): void {
  desktopSize = size;
}

/** Open an app by id, optionally with arguments. Returns the window id, or null if unknown. */
export function launchApp(appId: string, args?: Record<string, string>): string | null {
  const app = getApp(appId);
  if (!app) return null;
  return useWindows
    .getState()
    .open(
      { appId, title: app.title, size: app.defaultSize, singleton: app.singleton, args },
      desktopSize,
    );
}
