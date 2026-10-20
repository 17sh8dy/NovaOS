import { describe, expect, it } from 'vitest';
import {
  DEFAULTS,
  WALLPAPERS,
  isWallpaperFileName,
  patchSection,
  sanitize,
  selectionColor,
  wallpaperFile,
} from './schema';
import { BASE_COLOR, wallpaperStyle } from './wallpaper';

const app = (over: Partial<typeof DEFAULTS.appearance>) => ({ ...DEFAULTS.appearance, ...over });
const none = { url: null, status: 'idle' as const };

describe('custom wallpaper references', () => {
  it('accepts only safe stored names', () => {
    for (const ok of [
      'a.png',
      'My Photo (1).jpg',
      'sun_set-2.WEBP',
      '(x).gif',
      'a.b.bmp',
      'z.jpeg',
    ])
      expect(isWallpaperFileName(ok), ok).toBe(true);
    for (const bad of [
      '',
      '.png',
      ' a.png',
      'a.exe',
      'a.svg',
      'a',
      '../a.png',
      'a/b.png',
      'a\\b.png',
      'a:b.png',
      'a.png.exe',
      'x'.repeat(90) + '.png',
      'é.png',
      null,
      5,
    ])
      expect(isWallpaperFileName(bad), String(bad)).toBe(false);
  });

  it('wallpaperFile only unwraps valid file: references', () => {
    expect(wallpaperFile('file:Sunset.png')).toBe('Sunset.png');
    expect(wallpaperFile('aurora')).toBeNull();
    expect(wallpaperFile('file:../../etc/passwd')).toBeNull();
    expect(wallpaperFile('file:a.exe')).toBeNull();
    expect(wallpaperFile('file:')).toBeNull();
  });

  it('sanitize keeps valid presets and files, and falls back on anything else', () => {
    expect(sanitize({ appearance: { wallpaper: 'ocean' } }).appearance.wallpaper).toBe('ocean');
    expect(sanitize({ appearance: { wallpaper: 'file:Mine.jpg' } }).appearance.wallpaper).toBe(
      'file:Mine.jpg',
    );
    for (const bad of [
      'file:../x.png',
      'file:x.exe',
      'url(evil)',
      'http://x/y.png',
      5,
      null,
      'file:a/b.png',
    ])
      expect(sanitize({ appearance: { wallpaper: bad } }).appearance.wallpaper).toBe('aurora');
  });

  it('fit, dim and selection settings are validated', () => {
    expect(sanitize({ appearance: { wallpaperFit: 'tile' } }).appearance.wallpaperFit).toBe('tile');
    expect(sanitize({ appearance: { wallpaperFit: 'zoom' } }).appearance.wallpaperFit).toBe('fill');
    expect(sanitize({ appearance: { wallpaperDim: 9 } }).appearance.wallpaperDim).toBe(0.6);
    expect(sanitize({ appearance: { wallpaperDim: -1 } }).appearance.wallpaperDim).toBe(0);
    expect(sanitize({ appearance: { selectionColor: '#ff0000' } }).appearance.selectionColor).toBe(
      '#ff0000',
    );
    expect(sanitize({ appearance: { selectionColor: 'accent' } }).appearance.selectionColor).toBe(
      'accent',
    );
    for (const bad of ['red', '#fff', 'url(x)', 5, null])
      expect(sanitize({ appearance: { selectionColor: bad } }).appearance.selectionColor).toBe(
        'accent',
      );
    expect(sanitize({ appearance: { selectionOpacity: 5 } }).appearance.selectionOpacity).toBe(0.5);
    expect(sanitize({ appearance: { selectionOpacity: 0 } }).appearance.selectionOpacity).toBe(
      0.05,
    );
    expect(sanitize({ appearance: { selectionEnabled: 'no' } }).appearance.selectionEnabled).toBe(
      true,
    );
    expect(
      patchSection(DEFAULTS, 'appearance', { selectionEnabled: false }).appearance.selectionEnabled,
    ).toBe(false);
  });

  it('selectionColor follows the accent unless a color is chosen', () => {
    expect(selectionColor(app({ accent: 'teal', selectionColor: 'accent' }))).toBe('#14b8a6');
    expect(selectionColor(app({ accent: '#112233', selectionColor: 'accent' }))).toBe('#112233');
    expect(selectionColor(app({ selectionColor: '#ff00aa' }))).toBe('#ff00aa');
  });
});

describe('wallpaperStyle', () => {
  it('uses the preset gradient, with an optional dimming layer', () => {
    expect(wallpaperStyle(app({ wallpaper: 'ocean' }), none)).toEqual({
      background: WALLPAPERS.ocean,
    });
    const dimmed = wallpaperStyle(app({ wallpaper: 'ocean', wallpaperDim: 0.3 }), none);
    expect(String(dimmed.background)).toContain('rgb(0 0 0 / 0.3)');
    expect(String(dimmed.background)).toContain(WALLPAPERS.ocean);
  });

  it('shows a custom image with the chosen fit', () => {
    const ready = { url: 'blob:abc', status: 'ready' as const };
    const fill = wallpaperStyle(app({ wallpaper: 'file:a.png', wallpaperFit: 'fill' }), ready);
    expect(fill).toMatchObject({
      backgroundImage: 'url("blob:abc")',
      backgroundSize: 'cover',
      backgroundRepeat: 'no-repeat',
      backgroundColor: BASE_COLOR,
    });
    expect(
      wallpaperStyle(app({ wallpaper: 'file:a.png', wallpaperFit: 'fit' }), ready).backgroundSize,
    ).toBe('contain');
    expect(
      wallpaperStyle(app({ wallpaper: 'file:a.png', wallpaperFit: 'stretch' }), ready)
        .backgroundSize,
    ).toBe('100% 100%');
    const tile = wallpaperStyle(app({ wallpaper: 'file:a.png', wallpaperFit: 'tile' }), ready);
    expect(tile.backgroundRepeat).toBe('repeat');
    expect(
      wallpaperStyle(app({ wallpaper: 'file:a.png', wallpaperFit: 'center' }), ready)
        .backgroundSize,
    ).toBe('auto');
  });

  it('layers the dim over the image', () => {
    const s = wallpaperStyle(app({ wallpaper: 'file:a.png', wallpaperDim: 0.4 }), {
      url: 'blob:abc',
      status: 'ready',
    });
    expect(String(s.backgroundImage)).toMatch(
      /^linear-gradient\(rgb\(0 0 0 \/ 0\.4\), rgb\(0 0 0 \/ 0\.4\)\), url\("blob:abc"\)$/,
    );
    expect(String(s.backgroundSize).split(',')).toHaveLength(2);
  });

  it('falls back safely: plain base while loading, the default wallpaper if the image is gone or broken', () => {
    expect(
      wallpaperStyle(app({ wallpaper: 'file:a.png' }), { url: null, status: 'loading' }),
    ).toEqual({ background: BASE_COLOR });
    expect(
      wallpaperStyle(app({ wallpaper: 'file:a.png' }), { url: null, status: 'failed' }),
    ).toEqual({ background: WALLPAPERS.aurora });
  });
});
