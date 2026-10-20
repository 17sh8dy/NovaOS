import { listApps } from '../../../kernel/appRegistry';
import type { IconSize, TaskbarPosition, TaskbarSize } from '../../../settings/schema';
import { useSettings } from '../../../settings/store';
import { Group, Page, Row, Segmented, Toggle } from '../ui';

export function DesktopTaskbar() {
  const { data, set } = useSettings();
  const d = data.desktop;
  const pinnable = listApps().filter((a) => a.pinned);

  const setPinned = (id: string, pinned: boolean) =>
    set('desktop', {
      unpinned: pinned ? d.unpinned.filter((x) => x !== id) : [...d.unpinned, id],
    });

  return (
    <Page title="Desktop & Taskbar" description="Where things live on your desktop.">
      <Group title="Taskbar">
        <Row label="Position">
          <Segmented<TaskbarPosition>
            label="Taskbar position"
            value={d.taskbarPosition}
            onChange={(v) => set('desktop', { taskbarPosition: v })}
            options={[
              { value: 'bottom', label: 'Bottom' },
              { value: 'top', label: 'Top' },
            ]}
          />
        </Row>
        <Row label="Size">
          <Segmented<TaskbarSize>
            label="Taskbar size"
            value={d.taskbarSize}
            onChange={(v) => set('desktop', { taskbarSize: v })}
            options={[
              { value: 'compact', label: 'Compact' },
              { value: 'default', label: 'Default' },
              { value: 'large', label: 'Large' },
            ]}
          />
        </Row>
      </Group>

      <Group title="Desktop icons">
        <Row label="Show desktop icons">
          <Toggle
            label="Show desktop icons"
            checked={d.showDesktopIcons}
            onChange={(v) => set('desktop', { showDesktopIcons: v })}
          />
        </Row>
        <p className="px-4 py-2 text-xs text-white/50">
          The Recycle Bin is always on the desktop, even when desktop icons are hidden.
        </p>
        <Row label="Icon size">
          <Segmented<IconSize>
            label="Icon size"
            value={d.iconSize}
            onChange={(v) => set('desktop', { iconSize: v })}
            options={[
              { value: 'small', label: 'Small' },
              { value: 'medium', label: 'Medium' },
              { value: 'large', label: 'Large' },
            ]}
          />
        </Row>
      </Group>

      <Group title="Pinned apps">
        <p className="px-4 py-2 text-xs text-white/50">
          Pinned apps appear on the taskbar and the desktop. Every app is always in the Start menu.
        </p>
        {pinnable.map((a) => (
          <Row key={a.id} label={a.title}>
            <Toggle
              label={`Pin ${a.title}`}
              checked={!d.unpinned.includes(a.id)}
              onChange={(v) => setPinned(a.id, v)}
            />
          </Row>
        ))}
      </Group>
    </Page>
  );
}
