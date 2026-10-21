import { describe, expect, it } from 'vitest';
import {
  BITS_CATALOG,
  BITS_DEFAULTS,
  BIT_LIMITS,
  activeBits,
  allBitsOff,
  anyBitOn,
  countBitsOn,
  sanitizeBits,
  type BitId,
} from './bits';
import { DEFAULTS, patchSection, sanitize } from './schema';

const ids = Object.keys(BITS_DEFAULTS) as BitId[];
const allOn = () => {
  const out = sanitizeBits({});
  for (const id of ids) (out[id] as { enabled: boolean }).enabled = true;
  return out;
};

describe('React Bits settings', () => {
  it('every effect is off out of the box', () => {
    expect(DEFAULTS.bits).toEqual(BITS_DEFAULTS);
    for (const id of ids) expect(BITS_DEFAULTS[id].enabled, id).toBe(false);
    expect(anyBitOn(BITS_DEFAULTS)).toBe(false);
  });

  it('sanitize turns any junk into the defaults', () => {
    for (const junk of [null, undefined, 5, 'x', [], { clickSpark: 'no' }, { clickSpark: [] }]) {
      expect(sanitizeBits(junk)).toEqual(BITS_DEFAULTS);
    }
    expect(sanitize({ bits: 'nope' }).bits).toEqual(BITS_DEFAULTS);
    expect(sanitize({}).bits).toEqual(BITS_DEFAULTS);
  });

  it('clamps numbers into range and snaps them to the slider step', () => {
    const b = sanitizeBits({
      clickSpark: { enabled: true, count: 999, length: -5, duration: 'fast' },
      glowCursor: { size: 123, intensity: 0.333 },
      dock: { magnify: 50 },
      swarmCursor: { count: 7.6 },
    });
    expect(b.clickSpark.count).toBe(BIT_LIMITS.sparkCount.max);
    expect(b.clickSpark.length).toBe(BIT_LIMITS.sparkLength.min);
    expect(b.clickSpark.duration).toBe(BITS_DEFAULTS.clickSpark.duration);
    expect(b.glowCursor.size).toBe(120); // snapped to a multiple of 20 from 80
    expect(b.glowCursor.intensity).toBe(0.35);
    expect(b.dock.magnify).toBe(BIT_LIMITS.dockMagnify.max);
    expect(b.swarmCursor.count).toBe(8);
    for (const bad of [NaN, Infinity, -Infinity]) {
      expect(sanitizeBits({ magnet: { radius: bad } }).magnet.radius).toBe(120);
    }
  });

  it('accepts only accent or a full #rrggbb color', () => {
    expect(sanitizeBits({ clickSpark: { color: '#FFAA00' } }).clickSpark.color).toBe('#ffaa00');
    expect(sanitizeBits({ clickSpark: { color: 'accent' } }).clickSpark.color).toBe('accent');
    for (const bad of ['red', '#fff', '#12345', 'url(x)', 'var(--x)', 5, null]) {
      expect(sanitizeBits({ clickSpark: { color: bad } }).clickSpark.color, String(bad)).toBe(
        'accent',
      );
    }
  });

  it('text targets keep only known, unique names', () => {
    const b = sanitizeBits({
      shinyText: { targets: ['icons', 'icons', 'clock', 'nope', 5] },
      echoText: { targets: 'titles' },
    });
    expect(b.shinyText.targets).toEqual(['icons', 'clock']);
    expect(b.echoText.targets).toEqual(BITS_DEFAULTS.echoText.targets);
    expect(sanitizeBits({ foldText: { targets: [] } }).foldText.targets).toEqual([]);
  });

  it('list style must be one of the known styles', () => {
    expect(sanitizeBits({ animatedList: { style: 'fade' } }).animatedList.style).toBe('fade');
    expect(sanitizeBits({ animatedList: { style: 'explode' } }).animatedList.style).toBe('slide');
  });

  it('drops unknown keys and survives a round trip through the whole settings', () => {
    const messy = sanitizeBits({ clickSpark: { enabled: true, evil: '<script>' }, other: 1 });
    expect(messy.clickSpark).not.toHaveProperty('evil');
    expect(messy).not.toHaveProperty('other');
    const set = patchSection(DEFAULTS, 'bits', allOn());
    expect(sanitize(JSON.parse(JSON.stringify(set))).bits).toEqual(set.bits);
  });

  it('every catalog entry matches a real effect and its sliders sit inside their limits', () => {
    expect(BITS_CATALOG.map((b) => b.id).sort()).toEqual([...ids].sort());
    for (const def of BITS_CATALOG) {
      const cur = BITS_DEFAULTS[def.id] as unknown as Record<string, unknown>;
      expect(def.title.length, def.id).toBeGreaterThan(0);
      for (const p of def.params) {
        expect(cur, `${def.id}.${p.key}`).toHaveProperty(p.key);
        if (p.kind === 'slider') {
          const v = cur[p.key] as number;
          expect(v).toBeGreaterThanOrEqual(p.range.min);
          expect(v).toBeLessThanOrEqual(p.range.max);
        }
      }
    }
  });

  it('counts and clears effects, keeping the tweaks', () => {
    const on = sanitizeBits({
      clickSpark: { enabled: true, count: 12 },
      magnet: { enabled: true },
    });
    expect(countBitsOn(on)).toBe(2);
    const off = allBitsOff(on);
    expect(countBitsOn(off)).toBe(0);
    expect(off.clickSpark.count).toBe(12);
  });
});

describe('accessibility always wins', () => {
  it('does nothing to the settings when neither accessibility option is on', () => {
    const b = allOn();
    expect(activeBits(b, { reduceMotion: false, highContrast: false })).toBe(b);
  });

  it('Reduce motion switches off everything that moves, but keeps still Echo Text', () => {
    const r = activeBits(allOn(), { reduceMotion: true, highContrast: false });
    for (const id of ids) {
      expect(r[id].enabled, id).toBe(id === 'echoText');
    }
  });

  it('High contrast switches off effects that dim or wash out text and icons', () => {
    const r = activeBits(allOn(), { reduceMotion: false, highContrast: true });
    for (const id of ['chromaGrid', 'shinyText', 'echoText', 'foldText'] as const) {
      expect(r[id].enabled, id).toBe(false);
    }
    expect(r.clickSpark.enabled).toBe(true);
  });

  it('never changes the saved settings', () => {
    const b = allOn();
    const before = JSON.stringify(b);
    activeBits(b, { reduceMotion: true, highContrast: true });
    expect(JSON.stringify(b)).toBe(before);
  });
});
