import { LIMITS } from '../../../settings/schema';
import { useSettings } from '../../../settings/store';
import { Group, Page, Row, Slider, Toggle } from '../ui';

export function Accessibility() {
  const { data, set } = useSettings();
  const x = data.accessibility;
  return (
    <Page title="Accessibility" description="Make NovaOS easier to see and use.">
      <Group title="Size">
        <Row label="Interface size" hint="Scales text and spacing across NovaOS.">
          <Slider
            label="Interface size"
            {...LIMITS.uiScale}
            value={x.uiScale}
            onChange={(v) => set('accessibility', { uiScale: v })}
            format={(v) => `${Math.round(v * 100)}%`}
          />
        </Row>
      </Group>
      <Group title="Vision & motion">
        <Row label="High contrast" hint="Solid window backgrounds and brighter borders and text.">
          <Toggle
            label="High contrast"
            checked={x.highContrast}
            onChange={(v) => set('accessibility', { highContrast: v })}
          />
        </Row>
        <Row label="Reduce motion" hint="Turns off window animations and transitions.">
          <Toggle
            label="Reduce motion"
            checked={x.reduceMotion}
            onChange={(v) => set('accessibility', { reduceMotion: v })}
          />
        </Row>
      </Group>
    </Page>
  );
}
