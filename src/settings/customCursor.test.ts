import { describe, expect, it } from 'vitest';
import { decodeCursorFile, framePose, parseAni, parseCur } from './customCursor';
import { DEFAULTS, isCursorFileName, sanitize } from './schema';

const le16 = (n: number) => [n & 255, (n >> 8) & 255];
const le32 = (n: number) => [n & 255, (n >> 8) & 255, (n >> 16) & 255, (n >>> 24) & 255];
const ascii = (s: string) => [...s].map((c) => c.charCodeAt(0));

/** A minimal Windows .cur with the given images: [width, height, hotspotX, hotspotY]. */
function cur(images: [number, number, number, number][]): Uint8Array {
  const header = [0, 0, 2, 0, ...le16(images.length)];
  const dir: number[] = [];
  const body: number[] = [];
  let offset = 6 + images.length * 16;
  for (const [w, h, hx, hy] of images) {
    const data = new Array(40).fill(1); // stand-in image bytes
    dir.push(w === 256 ? 0 : w, h === 256 ? 0 : h, 0, 0, ...le16(hx), ...le16(hy));
    dir.push(...le32(data.length), ...le32(offset));
    body.push(...data);
    offset += data.length;
  }
  return Uint8Array.from([...header, ...dir, ...body]);
}

const chunk = (id: string, data: number[]) => [
  ...ascii(id),
  ...le32(data.length),
  ...data,
  ...(data.length % 2 ? [0] : []),
];

/** A minimal .ani: `frames` cursors, with optional per-step rates and a custom sequence. */
function ani(opts: { frames: Uint8Array[]; rates?: number[]; seq?: number[]; flags?: number }) {
  const steps = opts.seq?.length ?? opts.frames.length;
  const anih = [
    ...le32(36),
    ...le32(opts.frames.length),
    ...le32(steps),
    ...le32(32),
    ...le32(32),
    ...le32(32),
    ...le32(1),
    ...le32(6),
    ...le32(opts.flags ?? 1),
  ];
  const fram = [...ascii('fram'), ...opts.frames.flatMap((f) => chunk('icon', [...f]))];
  const body = [
    ...ascii('ACON'),
    ...chunk('anih', anih),
    ...(opts.rates ? chunk('rate', opts.rates.flatMap(le32)) : []),
    ...(opts.seq ? chunk('seq ', opts.seq.flatMap(le32)) : []),
    ...chunk('LIST', fram),
  ];
  return Uint8Array.from([...ascii('RIFF'), ...le32(body.length), ...body]);
}

describe('.cur files', () => {
  it('reads the size and the hotspot', () => {
    const f = parseCur(cur([[32, 32, 5, 7]]));
    expect(f).toMatchObject({ width: 32, height: 32, hotspot: [5, 7], mime: 'image/x-icon' });
  });

  it('with several sizes inside, uses the largest one and its own hotspot', () => {
    const f = parseCur(
      cur([
        [16, 16, 1, 1],
        [48, 48, 9, 11],
        [32, 32, 4, 4],
      ]),
    );
    expect(f).toMatchObject({ width: 48, height: 48, hotspot: [9, 11] });
  });

  it('treats a 0 size byte as 256', () => {
    expect(parseCur(cur([[256, 256, 3, 3]]))).toMatchObject({ width: 256, height: 256 });
  });

  it('never puts the hotspot outside the image', () => {
    expect(parseCur(cur([[16, 16, 900, 900]]))?.hotspot).toEqual([15, 15]);
  });

  it('refuses anything that is not a well-formed cursor', () => {
    const good = cur([[32, 32, 1, 1]]);
    expect(parseCur(new Uint8Array())).toBeNull();
    expect(parseCur(Uint8Array.from([0, 0, 1, 0, 1, 0, ...new Array(40).fill(0)]))).toBeNull(); // .ico
    expect(parseCur(good.subarray(0, 12))).toBeNull(); // cut off in the directory
    expect(parseCur(good.subarray(0, good.length - 10))).toBeNull(); // image data cut short
    const huge = Uint8Array.from(good);
    huge[4] = 255; // claims 255 images in a tiny file
    expect(parseCur(huge)).toBeNull();
    const zeroSize = Uint8Array.from(good);
    zeroSize.set(le32(0), 14);
    expect(parseCur(zeroSize)).toBeNull();
  });
});

describe('.ani files', () => {
  const a = cur([[32, 32, 2, 3]]);
  const b = cur([[32, 32, 4, 5]]);

  it('reads every frame with its own hotspot', () => {
    const r = parseAni(ani({ frames: [a, b] }));
    expect(r?.frames.map((f) => f.hotspot)).toEqual([
      [2, 3],
      [4, 5],
    ]);
    expect(r?.sequence).toEqual([0, 1]);
  });

  it('uses the default rate (jiffies of 1/60 s) when there is no rate chunk', () => {
    expect(parseAni(ani({ frames: [a, b] }))?.delays).toEqual([100, 100]);
  });

  it('honours per-step rates and a custom sequence', () => {
    const r = parseAni(ani({ frames: [a, b], rates: [3, 30, 6], seq: [0, 1, 0] }));
    expect(r?.sequence).toEqual([0, 1, 0]);
    expect(r?.delays).toEqual([50, 500, 100]);
  });

  it('keeps delays within a sane range', () => {
    const r = parseAni(ani({ frames: [a, b], rates: [1, 9999] }));
    expect(r?.delays[0]).toBe(33);
    expect(r?.delays[1]).toBe(2000);
  });

  it('refuses broken or unsupported files instead of guessing', () => {
    const good = ani({ frames: [a, b] });
    expect(parseAni(new Uint8Array())).toBeNull();
    expect(parseAni(Uint8Array.from([...ascii('RIFF'), ...le32(4), ...ascii('WAVE')]))).toBeNull();
    expect(parseAni(good.subarray(0, good.length - 20))).toBeNull(); // chunk runs past the end
    expect(parseAni(ani({ frames: [a, b], flags: 0 }))).toBeNull(); // raw-bitmap frames
    expect(parseAni(ani({ frames: [a], seq: [0, 7] }))).toBeNull(); // sequence points at a missing frame
    expect(parseAni(ani({ frames: [Uint8Array.from([1, 2, 3, 4, 5, 6, 7, 8])] }))).toBeNull(); // frame is not a .cur
  });

  it('caps the number of frames', () => {
    expect(parseAni(ani({ frames: Array.from({ length: 129 }, () => a) }))).toBeNull();
  });
});

describe('decodeCursorFile', () => {
  const png = (w: number, h: number) =>
    Uint8Array.from([
      0x89,
      0x50,
      0x4e,
      0x47,
      0x0d,
      0x0a,
      0x1a,
      0x0a,
      ...[0, 0, 0, 13],
      ...ascii('IHDR'),
      0,
      0,
      (w >> 8) & 255,
      w & 255,
      0,
      0,
      (h >> 8) & 255,
      h & 255,
    ]);

  it('reads PNG sizes from the header', () => {
    const d = decodeCursorFile('a.png', png(48, 64));
    expect(d?.frames[0]).toMatchObject({ width: 48, height: 64, hotspot: null, mime: 'image/png' });
    expect(d?.sequence).toEqual([0]);
  });

  it('reads GIF sizes from the header', () => {
    const gif = Uint8Array.from([...ascii('GIF89a'), ...le16(24), ...le16(20), 0, 0, 0]);
    expect(decodeCursorFile('a.GIF', gif)?.frames[0]).toMatchObject({ width: 24, height: 20 });
  });

  it('refuses nonsense, absurd sizes and unknown types', () => {
    expect(decodeCursorFile('a.png', Uint8Array.from([1, 2, 3]))).toBeNull();
    expect(decodeCursorFile('a.png', png(0, 10))).toBeNull();
    expect(decodeCursorFile('a.png', png(5000, 10))).toBeNull();
    expect(decodeCursorFile('a.svg', Uint8Array.from(ascii('<svg/>')))).toBeNull();
    expect(decodeCursorFile('a.exe', Uint8Array.from([0x4d, 0x5a]))).toBeNull();
  });

  it('decodes a .cur as a single still frame', () => {
    const d = decodeCursorFile('a.cur', cur([[32, 32, 1, 2]]));
    expect(d?.frames).toHaveLength(1);
    expect(d?.frames[0].hotspot).toEqual([1, 2]);
  });
});

describe('how a cursor is drawn', () => {
  const f = { width: 32, height: 32, hotspot: [4, 6] as [number, number] };

  it('the default pointer size shows a cursor at its natural size', () => {
    expect(framePose(f, 32, 'topleft')).toEqual({ width: 32, height: 32, hotspot: [4, 6] });
  });

  it('scales the image and its hotspot together', () => {
    expect(framePose(f, 48, 'topleft')).toEqual({ width: 48, height: 48, hotspot: [6, 9] });
    expect(framePose(f, 20, 'topleft').width).toBe(20);
  });

  it('never draws bigger than 128 px', () => {
    const p = framePose({ width: 256, height: 256, hotspot: [0, 0] }, 48, 'topleft');
    expect(p.width).toBeLessThanOrEqual(128);
    expect(p.height).toBeLessThanOrEqual(128);
  });

  it('PNG and GIF cursors point where you chose; .cur files point where they say', () => {
    const still = { width: 40, height: 20, hotspot: null };
    expect(framePose(still, 32, 'topleft').hotspot).toEqual([0, 0]);
    expect(framePose(still, 32, 'center').hotspot).toEqual([20, 10]);
    expect(framePose(f, 32, 'center').hotspot).toEqual([4, 6]);
  });
});

describe('the stored cursor file name', () => {
  it('accepts only safe cursor names', () => {
    for (const ok of ['arrow.cur', 'Fancy Arrow (2).ani', 'a-b_c.PNG', 'x.gif'])
      expect(isCursorFileName(ok), ok).toBe(true);
    for (const bad of [
      '',
      '../x.cur',
      'a/b.cur',
      'a\\b.cur',
      'x.exe',
      'x.cur.exe',
      'x.svg',
      '.cur',
      ' x.cur',
      `${'a'.repeat(90)}.cur`,
      5,
      null,
    ])
      expect(isCursorFileName(bad), String(bad)).toBe(false);
  });

  it('settings drop a bad cursor file name and keep a good one', () => {
    expect(sanitize({ cursor: { customFile: 'arrow.cur' } }).cursor.customFile).toBe('arrow.cur');
    expect(sanitize({ cursor: { customFile: '../../evil.cur' } }).cursor.customFile).toBe('');
    expect(sanitize({ cursor: { customFile: 'x.exe' } }).cursor.customFile).toBe('');
    expect(DEFAULTS.cursor.customFile).toBe('');
    expect(sanitize({ cursor: { customHotspot: 'middle' } }).cursor.customHotspot).toBe('topleft');
    expect(sanitize({ cursor: { customEverywhere: 'yes' } }).cursor.customEverywhere).toBe(false);
  });
});
