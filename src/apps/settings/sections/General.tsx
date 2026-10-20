import { useDisplay } from '../../../shell/displayMode';
import { formatClock } from '../../../shell/Taskbar';
import { useSettings } from '../../../settings/store';
import { Group, Page, Row, Segmented, Toggle, Kbd } from '../ui';

export function General() {
  const { data, set } = useSettings();
  const { general } = data;
  const { mode, setMode, error } = useDisplay();
  const preview = formatClock(new Date(), general);

  return (
    <Page title="General" description="How NovaOS starts and how it shows the time.">
      <Group title="Display">
        <Row
          label="Display mode"
          hint="Windowed runs in a normal window. Immersive fills the screen with the NovaOS desktop."
        >
          <Segmented<'windowed' | 'immersive' | 'shell'>
            label="Display mode"
            value={mode}
            onChange={(v) => void setMode(v)}
            options={[
              { value: 'windowed', label: 'Windowed' },
              { value: 'immersive', label: 'Immersive' },
              {
                value: 'shell',
                label: 'Shell (soon)',
                disabled: true,
                title: 'Shell Mode is not available in this release',
              },
            ]}
          />
        </Row>
        {error && <p className="px-4 py-2 text-xs text-rose-400">{error}</p>}
        <Row
          label="Start in Immersive Mode"
          hint="Open NovaOS fullscreen every time. It is never a trap: leave any time."
        >
          <Toggle
            label="Start in Immersive Mode"
            checked={general.startInImmersive}
            onChange={(v) => set('general', { startInImmersive: v })}
          />
        </Row>
        <p className="px-4 py-2 text-xs text-white/50">
          Leave Immersive Mode with <Kbd>Ctrl</Kbd>+<Kbd>Alt</Kbd>+<Kbd>Q</Kbd>, the power button on
          the taskbar, or the Start menu.
        </p>
      </Group>

      <Group title="Clock">
        <Row label="Time format">
          <Segmented
            label="Time format"
            value={general.clockFormat}
            onChange={(v) => set('general', { clockFormat: v })}
            options={[
              { value: '12h', label: '12-hour' },
              { value: '24h', label: '24-hour' },
            ]}
          />
        </Row>
        <Row label="Show seconds">
          <Toggle
            label="Show seconds"
            checked={general.showSeconds}
            onChange={(v) => set('general', { showSeconds: v })}
          />
        </Row>
        <Row label="Show date">
          <Toggle
            label="Show date"
            checked={general.showDate}
            onChange={(v) => set('general', { showDate: v })}
          />
        </Row>
        <Row label="Preview">
          <span data-testid="clock-preview" className="text-sm text-white/80 tabular-nums">
            {preview.time}
            {general.showDate && ` · ${preview.date}`}
          </span>
        </Row>
      </Group>
    </Page>
  );
}
