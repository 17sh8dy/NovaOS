import { afterEach, describe, expect, it, vi } from 'vitest';
import { BUILTIN_LOOKS } from './builtinPresets';
import { cursorCss, cursorSvg } from './cursor';
import { windowMotion } from './motion';
import { isPinned, movePinned, pinsFor, setPinned } from './pins';
import {
  DEFAULTS,
  DEFAULT_PINNED,
  FONTS,
  LIMITS,
  MAX_PRESETS,
  applyLook,
  cleanName,
  exportLook,
  hexToRgbTriplet,
  importLook,
  patchSection,
  pickPreset,
  sanitize,
  taskbarReserved,
  type SettingsData,
} from './schema';
import { playSound, shouldPlay, soundPlan } from './sounds';
import { themeAttrs, themeStyle } from './theme';

const none = { url: null, status: 'idle' as const };

describe('new settings sections are validated', () => {
  it('defaults are already valid (sanitize is a fixed point)', () => {
    expect(sanitize(DEFAULTS)).toEqual(DEFAULTS);
    expect(sanitize(JSON.parse(JSON.stringify(DEFAULTS)))).toEqual(DEFAULTS);
  });

  it('enums, booleans, colors and numbers for windows, motion, cursor, sounds, interface', () => {
    const out = sanitize({
      windows: { shadow: 'strong', titleBar: 'huge', activeBorder: 'off' },
      motion: { speed: 99, windowAnimation: 'slide' },
      cursor: {
        style: 'dot',
        color: '#abcdef',
        outline: 'red',
        size: 1,
        ripple: 'yes',
        rippleColor: '#00ff00',
      },
      sounds: { enabled: true, volume: 5, pack: 'retro', clicks: 1 },
      interface: { font: 'comic' },
      appearance: { surfaceColor: '#102030' },
      desktop: {
        taskbarStyle: 'floating',
        taskbarOpacity: 0,
        taskbarLabels: false,
        iconSpacing: 'roomy',
        iconLabels: false,
      },
    });
    expect(out.windows).toEqual({ shadow: 'strong', titleBar: 'default', activeBorder: 'off' });
    expect(out.motion).toEqual({ speed: LIMITS.motionSpeed.max, windowAnimation: 'slide' });
    expect(out.cursor).toEqual({
      engine: 'auto',
      style: 'dot',
      color: '#abcdef',
      outline: '#000000',
      size: LIMITS.cursorSize.min,
      ripple: false,
      rippleColor: '#00ff00',
      customFile: '',
      customHotspot: 'topleft',
      customEverywhere: false,
    });
    expect(out.sounds).toEqual({ enabled: true, volume: 1, pack: 'retro', clicks: false });
    expect(out.interface.font).toBe('system');
    expect(out.appearance.surfaceColor).toBe('#102030');
    expect(out.desktop).toMatchObject({
      taskbarStyle: 'floating',
      taskbarOpacity: LIMITS.taskbarOpacity.min,
      taskbarLabels: false,
      iconSpacing: 'roomy',
      iconLabels: false,
    });
  });

  it('rejects hostile colors anywhere a color is accepted', () => {
    const evil = [
      'url(javascript:1)',
      '#12345',
      '#gggggg',
      'red',
      '"><script>',
      7,
      null,
      '#12345678',
    ];
    for (const bad of evil) {
      const out = sanitize({
        appearance: { surfaceColor: bad },
        cursor: { color: bad, outline: bad, rippleColor: bad },
      });
      expect(out.appearance.surfaceColor).toBe(DEFAULTS.appearance.surfaceColor);
      expect(out.cursor.color).toBe(DEFAULTS.cursor.color);
      expect(out.cursor.outline).toBe(DEFAULTS.cursor.outline);
      expect(out.cursor.rippleColor).toBe('accent');
    }
  });

  it('studio favorites are bounded, well-formed ids', () => {
    const many = Array.from({ length: 200 }, (_, i) => `themes.x${i}`);
    expect(sanitize({ studio: { favorites: many } }).studio.favorites).toHaveLength(40);
    expect(
      sanitize({ studio: { favorites: ['themes.accent', 'Bad Id', '../x', 'themes.accent', 5] } })
        .studio.favorites,
    ).toEqual(['themes.accent']);
    expect(
      sanitize({ studio: { sidebar: 'hidden', previewPosition: 'left' } }).studio,
    ).toMatchObject({
      sidebar: 'full',
      previewPosition: 'top',
    });
  });
});

describe('presets and looks', () => {
  it('names are cleaned: control characters stripped, trimmed, bounded', () => {
    expect(cleanName('  Hi\u0000 there\n ')).toBe('Hi there');
    expect(cleanName('x'.repeat(100))).toHaveLength(40);
    for (const bad of ['', '   ', '\u0001\u0002', 5, null, undefined])
      expect(cleanName(bad)).toBeNull();
  });

  it('saved presets are sanitized, capped and unique', () => {
    const good = pickPreset(DEFAULTS);
    const presets = [
      { id: 'a-1', name: 'One', data: good },
      { id: 'a-1', name: 'Dup id', data: good },
      { id: 'Bad Id', name: 'Bad', data: good },
      { id: 'noname', name: '   ', data: good },
      {
        id: 'x',
        name: 'Evil',
        data: { appearance: { accent: 'javascript:1', surfaceColor: 'red' } },
      },
      'junk',
      null,
    ];
    const out = sanitize({ studio: { presets } }).studio.presets;
    expect(out.map((p) => p.id)).toEqual(['a-1', 'x']);
    expect(out[1].data.appearance.accent).toBe('violet');
    expect(out[1].data.appearance.surfaceColor).toBe(DEFAULTS.appearance.surfaceColor);
    const many = Array.from({ length: 80 }, (_, i) => ({ id: `p${i}`, name: `P${i}`, data: good }));
    expect(sanitize({ studio: { presets: many } }).studio.presets).toHaveLength(MAX_PRESETS);
  });

  it('a preset carries customization only, never general, files or studio settings', () => {
    const p = pickPreset(DEFAULTS);
    expect(Object.keys(p).sort()).toEqual(
      [
        'accessibility',
        'appearance',
        'bits',
        'cursor',
        'desktop',
        'interface',
        'motion',
        'sounds',
        'windows',
      ].sort(),
    );
    const base: SettingsData = patchSection(
      patchSection(DEFAULTS, 'general', { clockFormat: '24h' }),
      'studio',
      { favorites: ['themes.accent'] },
    );
    const look = pickPreset(patchSection(DEFAULTS, 'appearance', { accent: 'rose' }));
    const applied = applyLook(base, look);
    expect(applied.appearance.accent).toBe('rose');
    expect(applied.general.clockFormat).toBe('24h');
    expect(applied.studio.favorites).toEqual(['themes.accent']);
  });

  it('a look round-trips through export and import', () => {
    const data = pickPreset(
      patchSection(DEFAULTS, 'cursor', { style: 'crosshair', color: '#ff0000' }),
    );
    const back = importLook(exportLook('My Look', data));
    expect(back.name).toBe('My Look');
    expect(back.data).toEqual(data);
  });

  it('importing refuses non-looks and the future, and sanitizes what it accepts', () => {
    expect(() => importLook('nope{')).toThrow(/not valid JSON/);
    expect(() => importLook('{"app":"novaos","settings":{}}')).toThrow(/not a NovaOS look/);
    expect(() => importLook('{"app":"other","kind":"look","look":{}}')).toThrow(
      /not a NovaOS look/,
    );
    expect(() => importLook('{"app":"novaos","kind":"look","version":99,"look":{}}')).toThrow(
      /newer version/,
    );
    const out = importLook(
      '{"app":"novaos","kind":"look","version":2,"name":"\\u0000Hax","look":{"appearance":{"accent":"<b>"},"studio":{"favorites":["x"]}}}',
    );
    expect(out.name).toBe('Hax');
    expect(out.data.appearance.accent).toBe('violet');
    expect(out.data).not.toHaveProperty('studio');
  });

  it('every built-in look is valid, unique, and free of personal wallpapers', () => {
    const ids = new Set<string>();
    for (const l of BUILTIN_LOOKS) {
      expect(ids.has(l.id)).toBe(false);
      ids.add(l.id);
      expect(sanitize(l.data)).toMatchObject(l.data);
      expect(l.data.appearance.wallpaper.startsWith('file:')).toBe(false);
      expect(l.name.length).toBeGreaterThan(0);
    }
    expect(BUILTIN_LOOKS.length).toBeGreaterThanOrEqual(5);
    expect(BUILTIN_LOOKS[0].data).toEqual(pickPreset(DEFAULTS));
  });
});

describe('theme, cursor, motion, sounds', () => {
  it('themeStyle turns settings into CSS variables', () => {
    const data = patchSection(
      patchSection(
        patchSection(
          patchSection(DEFAULTS, 'appearance', {
            surfaceColor: '#102030',
            windowOpacity: 0.6,
            blur: 8,
            corners: 'round',
          }),
          'windows',
          { shadow: 'none', titleBar: 'large', activeBorder: 'off' },
        ),
        'desktop',
        { iconSpacing: 'roomy', taskbarOpacity: 0.9 },
      ),
      'interface',
      { font: 'mono' },
    );
    const s = themeStyle(data, none) as Record<string, string>;
    expect(s['--surface']).toBe('rgb(16 32 48 / 0.6)');
    expect(s['--blur']).toBe('8px');
    expect(s['--radius']).toBe('20px');
    expect(s['--shadow']).toBe('none');
    expect(s['--titlebar']).toBe('44px');
    expect(s['--active-border']).toBe('rgb(255 255 255 / 0.1)');
    expect(s['--icon-gap']).toBe('18px');
    expect(s['--taskbar-bg']).toBe('rgb(0 0 0 / 0.9)');
    expect(s.fontFamily).toBe(FONTS.mono);
    expect(s.cursor).toBeUndefined(); // system pointer: leave it alone
  });

  it('high contrast makes surfaces and the taskbar opaque with brighter borders', () => {
    const s = themeStyle(
      patchSection(DEFAULTS, 'accessibility', { highContrast: true }),
      none,
    ) as Record<string, string>;
    expect(s['--surface']).toBe('rgb(20 22 34 / 1)');
    expect(s['--taskbar-bg']).toBe('rgb(0 0 0 / 1)');
    expect(s['--border']).toBe('rgb(255 255 255 / 0.55)');
  });

  it('themeAttrs reports contrast, motion and custom cursor', () => {
    expect(themeAttrs(DEFAULTS)).toEqual({
      'data-contrast': 'normal',
      'data-motion': 'normal',
      'data-cursor': 'system',
    });
    const on = patchSection(
      patchSection(DEFAULTS, 'accessibility', { highContrast: true, reduceMotion: true }),
      'cursor',
      { style: 'dot' },
    );
    expect(themeAttrs(on)).toEqual({
      'data-contrast': 'high',
      'data-motion': 'reduced',
      'data-cursor': 'custom',
    });
  });

  it('draws each cursor style in the chosen colors and size, with a sensible hotspot', () => {
    expect(cursorSvg(DEFAULTS.cursor)).toBeNull();
    expect(cursorCss(DEFAULTS.cursor)).toBeNull();
    for (const style of ['classic', 'dot', 'crosshair'] as const) {
      const c = { ...DEFAULTS.cursor, style, color: '#ff00aa', outline: '#001122', size: 40 };
      const svg = cursorSvg(c) as string;
      expect(svg).toContain('#ff00aa');
      expect(svg).toContain('#001122');
      expect(svg).toContain('width="40"');
      const css = cursorCss(c) as string;
      expect(css).toMatch(/^url\("data:image\/svg\+xml,[^"]+"\) \d+ \d+, auto$/);
      expect(css).not.toContain('<'); // the SVG is URL-encoded
    }
    expect(cursorCss({ ...DEFAULTS.cursor, style: 'classic', size: 32 })).toMatch(/\) 4 2, auto$/);
    expect(cursorCss({ ...DEFAULTS.cursor, style: 'dot', size: 32 })).toMatch(/\) 16 16, auto$/);
  });

  it('cursor SVG can never carry hostile markup, because colors are validated first', () => {
    const c = sanitize({
      cursor: { style: 'classic', color: '"/><script>alert(1)</script>', outline: 'x' },
    }).cursor;
    expect(cursorSvg(c)).not.toMatch(/script|alert/);
  });

  it('window motion honours the animation, speed and minimized state', () => {
    const scale = windowMotion('scale', false, 1);
    expect(scale.initial).toMatchObject({ opacity: 0 });
    expect(scale.animate).toMatchObject({ opacity: 1, scale: 1 });
    expect(windowMotion('scale', true, 1).animate).toMatchObject({ opacity: 0, scale: 0.9 });
    expect(windowMotion('none', false, 1).transition.duration).toBe(0);
    expect(windowMotion('slide', false, 1).initial).toMatchObject({ y: 18 });
    expect(windowMotion('fade', true, 1).animate).toEqual({ opacity: 0 });
    expect(windowMotion('scale', false, 2).transition.duration).toBeCloseTo(
      windowMotion('scale', false, 1).transition.duration / 2,
    );
    expect(windowMotion('scale', false, 0).transition.duration).toBeGreaterThan(0); // never divides by zero
  });

  it('sound plans differ by pack and event, and are pure', () => {
    expect(soundPlan('click', 'soft')).toHaveLength(1);
    expect(soundPlan('open', 'chime')).toHaveLength(2);
    expect(soundPlan('open', 'chime')[1].at).toBeGreaterThan(0);
    expect(soundPlan('open', 'soft')).toEqual(soundPlan('open', 'soft'));
    for (const n of soundPlan('close', 'retro')) {
      expect(n.freq).toBeGreaterThan(100);
      expect(n.dur).toBeGreaterThan(0);
    }
  });

  it('sounds stay silent unless enabled; click sounds need their own switch', () => {
    const off = { ...DEFAULTS.sounds };
    expect(shouldPlay('open', off)).toBe(false);
    expect(shouldPlay('open', { ...off, enabled: true })).toBe(true);
    expect(shouldPlay('open', { ...off, enabled: true, volume: 0 })).toBe(false);
    expect(shouldPlay('click', { ...off, enabled: true })).toBe(false);
    expect(shouldPlay('click', { ...off, enabled: true, clicks: true })).toBe(true);
    expect(playSound('open', off)).toBe(false);
  });

  describe('with a fake audio device', () => {
    afterEach(() => vi.unstubAllGlobals());

    it('plays every note of the plan at the chosen waveform and volume', () => {
      const osc = () => ({
        type: '',
        frequency: { value: 0 },
        connect: vi.fn(function (this: unknown, n: unknown) {
          return n;
        }),
        start: vi.fn(),
        stop: vi.fn(),
      });
      const oscs: ReturnType<typeof osc>[] = [];
      const gainNode = () => ({
        gain: { setValueAtTime: vi.fn(), exponentialRampToValueAtTime: vi.fn() },
        connect: vi.fn(function (this: unknown) {
          return this;
        }),
      });
      class FakeCtx {
        currentTime = 0;
        state = 'running';
        destination = {};
        createOscillator() {
          const o = osc();
          o.connect = vi.fn(() => gainNode());
          oscs.push(o);
          return o;
        }
        createGain() {
          return gainNode();
        }
        resume() {}
      }
      vi.stubGlobal('AudioContext', FakeCtx);
      Object.defineProperty(window, 'AudioContext', { value: FakeCtx, configurable: true });
      const s = { ...DEFAULTS.sounds, enabled: true, pack: 'retro' as const };
      expect(playSound('open', s)).toBe(true);
      expect(oscs).toHaveLength(soundPlan('open', 'retro').length);
      expect(oscs[0].type).toBe('square');
      expect(oscs[0].start).toHaveBeenCalled();
      expect(oscs[0].stop).toHaveBeenCalled();
    });
  });
});

describe('pins', () => {
  it('pins at the end, unpins, and ignores no-ops', () => {
    let d = patchSection(DEFAULTS, 'desktop', { taskbarPinned: ['files'] });
    expect(isPinned(d, 'taskbar', 'files')).toBe(true);
    expect(isPinned(d, 'desktop', 'files')).toBe(true); // defaults still pin it on the desktop
    const same = setPinned(d, 'taskbar', 'files', true);
    expect(same).toBe(d);
    d = setPinned(d, 'taskbar', 'notes', true);
    expect(pinsFor(d, 'taskbar')).toEqual(['files', 'notes']);
    d = setPinned(d, 'taskbar', 'files', false);
    expect(pinsFor(d, 'taskbar')).toEqual(['notes']);
    expect(setPinned(d, 'taskbar', 'nope', false)).toBe(d);
  });

  it('taskbar and desktop pins are independent', () => {
    const d = setPinned(DEFAULTS, 'desktop', 'files', false);
    expect(isPinned(d, 'desktop', 'files')).toBe(false);
    expect(isPinned(d, 'taskbar', 'files')).toBe(true);
  });

  it('reorders within bounds', () => {
    const d = patchSection(DEFAULTS, 'desktop', { taskbarPinned: ['a', 'b', 'c'] });
    expect(pinsFor(movePinned(d, 'taskbar', 'b', -1), 'taskbar')).toEqual(['b', 'a', 'c']);
    expect(pinsFor(movePinned(d, 'taskbar', 'b', 1), 'taskbar')).toEqual(['a', 'c', 'b']);
    expect(movePinned(d, 'taskbar', 'a', -1)).toBe(d);
    expect(movePinned(d, 'taskbar', 'c', 1)).toBe(d);
    expect(movePinned(d, 'taskbar', 'zz', 1)).toBe(d);
  });

  it('refuses to pin past the limit', () => {
    const full = patchSection(DEFAULTS, 'desktop', {
      taskbarPinned: Array.from({ length: 40 }, (_, i) => `a${i}`),
    });
    expect(setPinned(full, 'taskbar', 'extra', true)).toBe(full);
  });

  it('the default pins include Nova Studio', () => {
    expect(DEFAULT_PINNED).toContain('studio');
  });
});

describe('small helpers', () => {
  it('taskbarReserved adds the gap only when floating', () => {
    expect(taskbarReserved({ taskbarSize: 'default', taskbarStyle: 'docked' })).toBe(48);
    expect(taskbarReserved({ taskbarSize: 'default', taskbarStyle: 'floating' })).toBe(64);
    expect(taskbarReserved({ taskbarSize: 'large', taskbarStyle: 'docked' })).toBe(58);
  });

  it('hexToRgbTriplet', () => {
    expect(hexToRgbTriplet('#141622')).toBe('20 22 34');
    expect(hexToRgbTriplet('#ffffff')).toBe('255 255 255');
    expect(hexToRgbTriplet('#000000')).toBe('0 0 0');
  });
});
