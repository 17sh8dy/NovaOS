import { describe, expect, it } from 'vitest';
import { reorderPinned } from './pins';
import { DEFAULTS } from './schema';

const order = (d: typeof DEFAULTS) => d.desktop.desktopPinned;

describe('reorderPinned (drag and drop)', () => {
  it('drops an app into the slot of the one it lands on, both directions', () => {
    const down = reorderPinned(DEFAULTS, 'desktop', 'files', 'calculator');
    expect(order(down)).toEqual(['notes', 'calculator', 'files', 'studio', 'settings']);
    const up = reorderPinned(DEFAULTS, 'desktop', 'settings', 'notes');
    expect(order(up)).toEqual(['files', 'settings', 'notes', 'calculator', 'studio']);
  });

  it('changes nothing for itself, unknown apps, or the other place', () => {
    expect(reorderPinned(DEFAULTS, 'desktop', 'files', 'files')).toBe(DEFAULTS);
    expect(reorderPinned(DEFAULTS, 'desktop', 'nope', 'files')).toBe(DEFAULTS);
    expect(reorderPinned(DEFAULTS, 'desktop', 'files', 'nope')).toBe(DEFAULTS);
    const d = reorderPinned(DEFAULTS, 'desktop', 'files', 'settings');
    expect(d.desktop.taskbarPinned).toEqual(DEFAULTS.desktop.taskbarPinned);
  });
});
