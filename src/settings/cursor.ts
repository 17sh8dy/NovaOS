import type { SettingsData } from './schema';

type Cursor = SettingsData['cursor'];

/**
 * A custom mouse cursor drawn as SVG. Colors are validated `#rrggbb` values (see `sanitize`), so
 * interpolating them is safe. Returns null for the system cursor.
 */
export function cursorSvg(c: Cursor): string | null {
  const { color, outline, size } = c;
  const open = `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 32 32">`;
  switch (c.style) {
    case 'classic':
      return `${open}<path d="M4 2 L4 24 L10 18.5 L14.5 28 L18 26.5 L13.6 17.2 L21.5 17.2 Z" fill="${color}" stroke="${outline}" stroke-width="1.6" stroke-linejoin="round"/></svg>`;
    case 'dot':
      return `${open}<circle cx="16" cy="16" r="8" fill="${color}" stroke="${outline}" stroke-width="2.2"/></svg>`;
    case 'crosshair': {
      const arms = 'M16 3 V12 M16 20 V29 M3 16 H12 M20 16 H29';
      return `${open}<path d="${arms}" stroke="${outline}" stroke-width="4.4" stroke-linecap="round" fill="none"/><path d="${arms}" stroke="${color}" stroke-width="2" stroke-linecap="round" fill="none"/></svg>`;
    }
    default:
      return null;
  }
}

/** Where the cursor "points", as a fraction of its size. */
const HOTSPOT: Record<Exclude<Cursor['style'], 'system'>, [number, number]> = {
  classic: [4 / 32, 2 / 32],
  dot: [0.5, 0.5],
  crosshair: [0.5, 0.5],
};

/** The value for the CSS `cursor` property, or null to leave the system cursor alone. */
export function cursorCss(c: Cursor): string | null {
  const svg = cursorSvg(c);
  if (!svg || c.style === 'system') return null;
  const [hx, hy] = HOTSPOT[c.style];
  const uri = `data:image/svg+xml,${encodeURIComponent(svg)}`;
  return `url("${uri}") ${Math.round(hx * c.size)} ${Math.round(hy * c.size)}, auto`;
}
