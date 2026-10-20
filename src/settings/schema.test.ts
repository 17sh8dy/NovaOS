import { beforeEach, describe, expect, it } from 'vitest';
import {
  ACCENTS,
  DEFAULTS,
  LIMITS,
  accentColor,
  exportSettings,
  importSettings,
  isHexColor,
  patchSection,
  sanitize,
} from './schema';
import { useSettings } from './store';

describe('sanitize', () => {
  it('turns anything non-object into the defaults', () => {
    for (const junk of [null, undefined, 5, 'x', [], true])
      expect(sanitize(junk)).toEqual(DEFAULTS);
  });

  it('keeps valid values and drops unknown keys', () => {
    const out = sanitize({
      general: { clockFormat: '24h', showSeconds: true, evil: 'x' },
      appearance: { accent: 'teal', wallpaper: 'ocean', corners: 'sharp' },
      surprise: { a: 1 },
    });
    expect(out.general).toEqual({ ...DEFAULTS.general, clockFormat: '24h', showSeconds: true });
    expect(out.appearance).toMatchObject({ accent: 'teal', wallpaper: 'ocean', corners: 'sharp' });
    expect(out).not.toHaveProperty('surprise');
    expect(out.general).not.toHaveProperty('evil');
  });

  it('single-click open defaults to off and only accepts a real boolean', () => {
    expect(DEFAULTS.general.singleClickOpen).toBe(false);
    expect(sanitize({ general: { singleClickOpen: true } }).general.singleClickOpen).toBe(true);
    for (const bad of ['true', 1, null, {}])
      expect(sanitize({ general: { singleClickOpen: bad } }).general.singleClickOpen).toBe(false);
  });

  it('rejects bad enums, wrong types and bad colours', () => {
    const out = sanitize({
      general: { clockFormat: '99h', startInImmersive: 'yes' },
      appearance: { accent: 'javascript:alert(1)', wallpaper: 'nope', corners: 3 },
      desktop: { taskbarPosition: 'left', iconSize: null },
    });
    expect(out.general.clockFormat).toBe('12h');
    expect(out.general.startInImmersive).toBe(false);
    expect(out.appearance.accent).toBe('violet');
    expect(out.appearance.wallpaper).toBe('aurora');
    expect(out.desktop.taskbarPosition).toBe('bottom');
  });

  it('accepts a custom hex accent only in #rrggbb form', () => {
    expect(sanitize({ appearance: { accent: '#12abEF' } }).appearance.accent).toBe('#12abEF');
    for (const bad of ['#fff', 'red', '#12345g', 'url(x)', '#1234567'])
      expect(sanitize({ appearance: { accent: bad } }).appearance.accent).toBe('violet');
    expect(isHexColor('#000000')).toBe(true);
    expect(accentColor('rose')).toBe(ACCENTS.rose);
    expect(accentColor('#abcdef')).toBe('#abcdef');
    expect(accentColor('garbage')).toBe(ACCENTS.violet);
  });

  it('clamps and snaps numbers to their limits', () => {
    const lo = sanitize({
      appearance: { windowOpacity: -3, blur: -9 },
      accessibility: { uiScale: 0 },
    });
    expect(lo.appearance.windowOpacity).toBe(LIMITS.windowOpacity.min);
    expect(lo.appearance.blur).toBe(LIMITS.blur.min);
    expect(lo.accessibility.uiScale).toBe(LIMITS.uiScale.min);
    const hi = sanitize({
      appearance: { windowOpacity: 9, blur: 999 },
      accessibility: { uiScale: 99 },
    });
    expect(hi.appearance.windowOpacity).toBe(LIMITS.windowOpacity.max);
    expect(hi.appearance.blur).toBe(LIMITS.blur.max);
    expect(hi.accessibility.uiScale).toBe(LIMITS.uiScale.max);
    expect(sanitize({ accessibility: { uiScale: 1.07 } }).accessibility.uiScale).toBe(1.05);
    for (const bad of [NaN, Infinity, '1', null])
      expect(sanitize({ accessibility: { uiScale: bad } }).accessibility.uiScale).toBe(1);
  });

  it('keeps only well-formed, de-duplicated, bounded unpinned ids', () => {
    const many = Array.from({ length: 300 }, (_, i) => `app-${i}`);
    expect(
      sanitize({ desktop: { unpinned: ['files', 'files', 'Bad Id', '../x', 5, 'notes'] } }).desktop
        .unpinned,
    ).toEqual(['files', 'notes']);
    expect(sanitize({ desktop: { unpinned: many } }).desktop.unpinned).toHaveLength(100);
    expect(sanitize({ desktop: { unpinned: 'files' } }).desktop.unpinned).toEqual([]);
  });

  it('patchSection validates the patch too', () => {
    const next = patchSection(DEFAULTS, 'appearance', { accent: 'nope', blur: 10 });
    expect(next.appearance.accent).toBe('violet');
    expect(next.appearance.blur).toBe(10);
    expect(next.general).toEqual(DEFAULTS.general);
  });
});

describe('export / import', () => {
  it('round-trips', () => {
    const data = patchSection(DEFAULTS, 'general', { clockFormat: '24h', startInImmersive: true });
    expect(importSettings(exportSettings(data))).toEqual(data);
  });

  it('refuses files that are not NovaOS exports, or are from the future', () => {
    expect(() => importSettings('{nope')).toThrow(/not valid JSON/);
    expect(() => importSettings('{"hello":1}')).toThrow(/not a NovaOS settings export/);
    expect(() => importSettings('[]')).toThrow(/not a NovaOS settings export/);
    expect(() => importSettings('{"app":"novaos","version":999,"settings":{}}')).toThrow(
      /newer version/,
    );
  });

  it('sanitizes what it imports', () => {
    const out = importSettings(
      '{"app":"novaos","version":1,"settings":{"appearance":{"accent":"<script>","blur":4}}}',
    );
    expect(out.appearance.accent).toBe('violet');
    expect(out.appearance.blur).toBe(4);
  });
});

describe('settings store', () => {
  beforeEach(() => {
    localStorage.clear();
    useSettings.setState({ data: DEFAULTS, loaded: false });
  });

  it('persists changes and reloads them', async () => {
    useSettings.getState().set('general', { clockFormat: '24h' });
    useSettings.getState().importJson(useSettings.getState().exportJson()); // forces an immediate save
    useSettings.setState({ data: DEFAULTS });
    await useSettings.getState().load();
    expect(useSettings.getState().data.general.clockFormat).toBe('24h');
    expect(useSettings.getState().loaded).toBe(true);
  });

  it('survives corrupt storage', async () => {
    localStorage.setItem('novaos:settings', '{{{ not json');
    await useSettings.getState().load();
    expect(useSettings.getState().data).toEqual(DEFAULTS);
    expect(useSettings.getState().loaded).toBe(true);
  });

  it('migrates the old theme-only record', async () => {
    localStorage.setItem('novaos:theme', JSON.stringify({ accent: 'rose', wallpaper: 'dusk' }));
    await useSettings.getState().load();
    expect(useSettings.getState().data.appearance).toMatchObject({
      accent: 'rose',
      wallpaper: 'dusk',
    });
  });

  it('reset restores defaults and saves them at once', async () => {
    useSettings.getState().set('desktop', { taskbarPosition: 'top' });
    useSettings.getState().reset();
    expect(useSettings.getState().data).toEqual(DEFAULTS);
    expect(
      JSON.parse(localStorage.getItem('novaos:settings') ?? '{}').desktop.taskbarPosition,
    ).toBe('bottom');
  });
});
