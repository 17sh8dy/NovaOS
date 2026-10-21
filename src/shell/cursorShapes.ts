import type { SettingsData } from '../settings/schema';

export type Shape =
  'arrow' | 'text' | 'pointer' | 'resize-ew' | 'resize-ns' | 'resize-nesw' | 'resize-nwse';

type Cursor = SettingsData['cursor'];

const NON_TEXT_INPUTS = new Set([
  'range',
  'checkbox',
  'radio',
  'color',
  'button',
  'submit',
  'reset',
  'file',
]);

/**
 * Which pointer shape belongs over this element. NovaOS draws its own pointer, so it picks the
 * shape itself, the way the system would: I-beam over text, a hand over links, arrows over resize
 * edges, the normal arrow everywhere else.
 */
export function shapeFor(target: EventTarget | null): Shape {
  if (!(target instanceof Element)) return 'arrow';

  const resize = target.closest('[class*="-resize"]');
  if (resize) {
    const m = /cursor-(ne|sw|nw|se|n|s|e|w)-resize/.exec(resize.getAttribute('class') ?? '');
    if (m) {
      if (m[1] === 'ne' || m[1] === 'sw') return 'resize-nesw';
      if (m[1] === 'nw' || m[1] === 'se') return 'resize-nwse';
      return m[1] === 'n' || m[1] === 's' ? 'resize-ns' : 'resize-ew';
    }
  }

  const field = target.closest('input, textarea, [contenteditable="true"], [contenteditable=""]');
  if (field && !(field as HTMLInputElement).disabled) {
    if (field instanceof HTMLInputElement && NON_TEXT_INPUTS.has(field.type)) return 'arrow';
    return 'text';
  }

  const link = target.closest('a[href], [role="link"], .cursor-pointer');
  if (link && !(link as HTMLButtonElement).disabled) return 'pointer';
  return 'arrow';
}

interface Drawing {
  svg: string;
  /** Where the pointer "points", in pixels from the image's top-left. */
  hotspot: [number, number];
}

/** Both colors are validated `#rrggbb` values (see `sanitize`), so they are safe to put in SVG. */
export function drawShape(
  shape: Shape,
  c: Pick<Cursor, 'color' | 'outline' | 'size' | 'style'>,
): Drawing {
  const { color, outline, size } = c;
  const open = `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 32 32">`;
  const k = size / 32;
  const center: [number, number] = [size / 2, size / 2];
  const stroked = (d: string, extra = '') =>
    `${open}<path d="${d}" fill="${color}" stroke="${outline}" stroke-width="1.6" stroke-linejoin="round" ${extra}/></svg>`;

  switch (shape) {
    case 'text': {
      const bar = 'M11 4 H21 M16 4 V28 M11 28 H21';
      return {
        svg: `${open}<path d="${bar}" stroke="${outline}" stroke-width="4.4" stroke-linecap="round" fill="none"/><path d="${bar}" stroke="${color}" stroke-width="2" stroke-linecap="round" fill="none"/></svg>`,
        hotspot: center,
      };
    }
    case 'pointer':
      return {
        svg: stroked(
          'M13 3.5 q-2 0 -2 2 V16 L9 14.2 q-2 -1.4 -3.4 .5 q-.5 1 .6 2.2 L12 25.5 q1.6 3 5 3 h4 q4 0 4 -4 V14 q0 -2 -2 -2 q-1 -1.5 -3 -.5 q-1 -1.5 -3 -.5 V5.5 q0 -2 -2 -2 z',
        ),
        hotspot: [Math.round(13 * k), Math.round(3.5 * k)],
      };
    case 'resize-ew':
    case 'resize-ns':
    case 'resize-nesw':
    case 'resize-nwse': {
      const rot =
        shape === 'resize-ns'
          ? 90
          : shape === 'resize-nesw'
            ? -45
            : shape === 'resize-nwse'
              ? 45
              : 0;
      const d = 'M3 16 L10 9 V13.5 H22 V9 L29 16 L22 23 V18.5 H10 V23 Z';
      return {
        svg: `${open}<g transform="rotate(${rot} 16 16)"><path d="${d}" fill="${color}" stroke="${outline}" stroke-width="1.6" stroke-linejoin="round"/></g></svg>`,
        hotspot: center,
      };
    }
    default: {
      // The normal pointer follows the user's chosen style (classic arrow unless they picked another).
      if (c.style === 'dot') {
        return {
          svg: `${open}<circle cx="16" cy="16" r="8" fill="${color}" stroke="${outline}" stroke-width="2.2"/></svg>`,
          hotspot: center,
        };
      }
      if (c.style === 'crosshair') {
        const arms = 'M16 3 V12 M16 20 V29 M3 16 H12 M20 16 H29';
        return {
          svg: `${open}<path d="${arms}" stroke="${outline}" stroke-width="4.4" stroke-linecap="round" fill="none"/><path d="${arms}" stroke="${color}" stroke-width="2" stroke-linecap="round" fill="none"/></svg>`,
          hotspot: center,
        };
      }
      return {
        svg: stroked('M4 2 L4 24 L10 18.5 L14.5 28 L18 26.5 L13.6 17.2 L21.5 17.2 Z'),
        hotspot: [Math.round(4 * k), Math.round(2 * k)],
      };
    }
  }
}

export const toDataUri = (svg: string): string => `data:image/svg+xml,${encodeURIComponent(svg)}`;
