import { Group, Kbd, Page, Row } from '../ui';

const GROUPS: { title: string; items: [string, string[]][] }[] = [
  {
    title: 'NovaOS',
    items: [
      ['Leave Immersive Mode (works from anywhere)', ['Ctrl', 'Alt', 'Q']],
      ['Launch the first Start-menu match', ['Enter']],
    ],
  },
  {
    title: 'Nova Files',
    items: [
      ['Open the selected item', ['Enter']],
      ['Rename', ['F2']],
      ['Delete to Recycle Bin', ['Delete']],
      ['Copy / Cut / Paste', ['Ctrl', 'C / X / V']],
      ['Go up a folder', ['Backspace']],
    ],
  },
  {
    title: 'Nova Studio',
    items: [
      ['Undo the last change', ['Ctrl', 'Z']],
      ['Redo', ['Ctrl', 'Shift', 'Z']],
    ],
  },
  {
    title: 'Pinning',
    items: [['Pin or unpin an app on the taskbar or desktop', ['Right-click']]],
  },
  { title: 'Nova Editor', items: [['Save now (it also autosaves)', ['Ctrl', 'S']]] },
];

export function Shortcuts() {
  return (
    <Page title="Keyboard shortcuts" description="Everything you can do from the keyboard.">
      {GROUPS.map((g) => (
        <Group key={g.title} title={g.title}>
          {g.items.map(([label, keys]) => (
            <Row key={label} label={label}>
              <span className="flex items-center gap-1">
                {keys.map((k, i) => (
                  <span key={k} className="flex items-center gap-1">
                    {i > 0 && <span className="text-white/30">+</span>}
                    <Kbd>{k}</Kbd>
                  </span>
                ))}
              </span>
            </Row>
          ))}
        </Group>
      ))}
    </Page>
  );
}
