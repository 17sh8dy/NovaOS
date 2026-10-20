import type { ComponentType } from 'react';
import type { LucideIcon } from 'lucide-react';

/** What a NovaOS application declares about itself. A future `.nova` package maps onto this. */
export interface AppManifest {
  id: string;
  title: string;
  icon: LucideIcon;
  component: ComponentType<{ windowId: string; args?: Record<string, string> }>;
  defaultSize: { width: number; height: number };
  singleton?: boolean;
  /** Pinned to the taskbar and shown on the desktop. */
  pinned?: boolean;
  /** Opened only by other apps (needs launch args); kept out of the Start menu. */
  hidden?: boolean;
}

const apps = new Map<string, AppManifest>();

export function registerApp(app: AppManifest): void {
  if (apps.has(app.id)) throw new Error(`app already registered: ${app.id}`);
  apps.set(app.id, app);
}

export const getApp = (id: string): AppManifest | undefined => apps.get(id);
export const listApps = (): AppManifest[] => [...apps.values()];

/** Test helper: start from a clean registry. */
export function resetAppRegistry(): void {
  apps.clear();
}
