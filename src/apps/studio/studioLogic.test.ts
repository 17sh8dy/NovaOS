import { describe, expect, it } from 'vitest';
import { CONTROLS, controlById, searchControls } from './controls';
import { COALESCE_MS, MAX_HISTORY, emptyHistory, record, stepRedo, stepUndo } from './history';
import { STUDIOS, resetAllCustomization, resetStudio, studioById } from './studios';
import { DEFAULTS, patchSection, type SettingsData } from '../../settings/schema';

const a: SettingsData = patchSection(DEFAULTS, 'appearance', { accent: 'rose' });
const b: SettingsData = patchSection(DEFAULTS, 'appearance', { accent: 'teal' });
const c: SettingsData = patchSection(DEFAULTS, 'appearance', { accent: 'amber' });

describe('undo / redo history', () => {
  it('records steps, undoes and redoes them in order', () => {
    let h = record(emptyHistory(), DEFAULTS, 'k1', 0);
    h = record(h, a, 'k2', 5000);
    const u1 = stepUndo(h, b)!;
    expect(u1.data).toBe(a);
    const u2 = stepUndo(u1.state, a)!;
    expect(u2.data).toBe(DEFAULTS);
    expect(stepUndo(u2.state, DEFAULTS)).toBeNull();
    const r1 = stepRedo(u2.state, DEFAULTS)!;
    expect(r1.data).toBe(a);
    const r2 = stepRedo(r1.state, a)!;
    expect(r2.data).toBe(b);
    expect(stepRedo(r2.state, b)).toBeNull();
  });

  it('a new change clears the redo stack', () => {
    let h = record(emptyHistory(), DEFAULTS, 'k1', 0);
    const u = stepUndo(h, a)!;
    expect(u.state.redo).toHaveLength(1);
    h = record(u.state, DEFAULTS, 'k2', 10_000);
    expect(h.redo).toHaveLength(0);
  });

  it('dragging one slider is a single undo step; other controls or a pause start a new one', () => {
    let h = record(emptyHistory(), DEFAULTS, 'windows.opacity', 0);
    h = record(h, a, 'windows.opacity', 100);
    h = record(h, b, 'windows.opacity', 300);
    expect(h.undo).toHaveLength(1);
    expect(h.undo[0]).toBe(DEFAULTS); // restores to before the whole drag
    h = record(h, c, 'windows.blur', 400);
    expect(h.undo).toHaveLength(2);
    h = record(h, c, 'windows.blur', 400 + COALESCE_MS + 1);
    expect(h.undo).toHaveLength(3);
  });

  it('is bounded', () => {
    let h = emptyHistory();
    for (let i = 0; i < MAX_HISTORY + 40; i++) h = record(h, DEFAULTS, `k${i}`, i * 10_000);
    expect(h.undo).toHaveLength(MAX_HISTORY);
  });
});

describe('studio resets', () => {
  it('resetting one studio restores only what it owns', () => {
    const messy = patchSection(
      patchSection(
        patchSection(DEFAULTS, 'appearance', {
          accent: 'rose',
          windowOpacity: 0.6,
          wallpaper: 'ocean',
        }),
        'cursor',
        { style: 'dot' },
      ),
      'general',
      { clockFormat: '24h' },
    );
    const out = resetStudio(messy, 'themes');
    expect(out.appearance.accent).toBe('violet');
    expect(out.appearance.wallpaper).toBe('aurora');
    expect(out.appearance.windowOpacity).toBe(0.6); // that belongs to the Windows studio
    expect(out.cursor.style).toBe('dot'); // another studio
    expect(out.general.clockFormat).toBe('24h'); // never customization
    expect(resetStudio(out, 'cursor').cursor.style).toBe('system');
  });

  it('reset all returns every customization to default but keeps personal data', () => {
    const messy = patchSection(
      patchSection(patchSection(DEFAULTS, 'appearance', { accent: 'rose', blur: 0 }), 'motion', {
        windowAnimation: 'none',
        speed: 2,
      }),
      'studio',
      { favorites: ['themes.accent'], presets: [{ id: 'x', name: 'X', data: DEFAULTS as never }] },
    );
    const out = resetAllCustomization(messy);
    expect(out.appearance).toEqual(DEFAULTS.appearance);
    expect(out.motion).toEqual(DEFAULTS.motion);
    expect(out.studio.favorites).toEqual(['themes.accent']);
    expect(out.studio.presets).toHaveLength(1);
  });

  it('every studio resets cleanly without touching other studios', () => {
    for (const s of STUDIOS) {
      const out = resetStudio(DEFAULTS, s.id);
      expect(out).toEqual(DEFAULTS);
      expect(studioById(s.id).title).toBe(s.title);
    }
  });
});

describe('control registry', () => {
  it('has unique, well-formed ids and a real home studio for each control', () => {
    const ids = new Set<string>();
    for (const c of CONTROLS) {
      expect(c.id).toMatch(/^[a-z0-9.-]{1,60}$/);
      expect(ids.has(c.id)).toBe(false);
      ids.add(c.id);
      expect(STUDIOS.some((s) => s.id === c.studio)).toBe(true);
      expect(c.title.length).toBeGreaterThan(0);
      expect(c.group.length).toBeGreaterThan(0);
      expect(c.keywords.length).toBeGreaterThan(0);
      expect(controlById(c.id)).toBe(c);
    }
  });

  it('every studio has controls', () => {
    for (const s of STUDIOS) expect(CONTROLS.some((c) => c.studio === s.id)).toBe(true);
    expect(CONTROLS.length).toBeGreaterThanOrEqual(40);
  });

  it('search finds controls across studios by title, hint, keyword and studio name', () => {
    const title = (id: string) => studioById(id as never).title;
    const ids = (q: string) => searchControls(q, title).map((c) => c.id);
    expect(ids('cursor speed')).toEqual([]); // honest: there is no OS mouse speed here
    expect(ids('blur')).toContain('windows.blur');
    expect(ids('taskbar transparency')).toContain('taskbar.opacity');
    expect(ids('ripple')).toEqual(expect.arrayContaining(['cursor.ripple', 'cursor.ripplecolor']));
    expect(ids('FONT')).toContain('interface.font');
    expect(ids('sounds')).toContain('sounds.enabled');
    expect(ids('   ')).toEqual([]);
    expect(ids('zzzznope')).toEqual([]);
    // multi-word queries must all match
    expect(ids('selection color')).toContain('desktop.selectioncolor');
    expect(ids('selection color')).not.toContain('desktop.icons');
  });
});
