import type { AppManifest } from '../kernel/appRegistry';
import { isPinned, type PinPlace } from '../settings/pins';
import type { SettingsData } from '../settings/schema';
import type { MenuItem } from './ContextMenu';

/** The right-click menu for an app: open, and pin or unpin it on the taskbar and the desktop. */
export function appMenuItems(
  app: AppManifest,
  data: SettingsData,
  actions: { open: () => void; pin: (place: PinPlace, id: string, pinned: boolean) => void },
): MenuItem[] {
  const t = isPinned(data, 'taskbar', app.id);
  const d = isPinned(data, 'desktop', app.id);
  return [
    { label: 'Open', onClick: actions.open },
    {
      label: t ? 'Unpin from taskbar' : 'Pin to taskbar',
      onClick: () => actions.pin('taskbar', app.id, !t),
    },
    {
      label: d ? 'Unpin from desktop' : 'Pin to desktop',
      onClick: () => actions.pin('desktop', app.id, !d),
    },
  ];
}
